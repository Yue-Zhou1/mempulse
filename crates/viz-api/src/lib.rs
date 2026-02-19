mod live_rpc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use live_rpc::{LiveRpcConfig, start_live_rpc_feed};
use replay::{ReplayMode, replay_frames};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{EventStore, InMemoryStorage, TxFeaturesRecord};
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct AppState {
    pub provider: Arc<dyn VizDataProvider>,
    pub downsample_limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayPoint {
    pub seq_hi: u64,
    pub timestamp_unix_ms: i64,
    pub pending_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PropagationEdge {
    pub source: String,
    pub destination: String,
    pub p50_delay_ms: u32,
    pub p99_delay_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeatureSummary {
    pub protocol: String,
    pub category: String,
    pub count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionSummary {
    pub hash: String,
    pub sender: String,
    pub nonce: u64,
    pub tx_type: u8,
    pub seen_unix_ms: i64,
    pub source_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamHello {
    pub event: String,
    pub message: String,
    pub replay_points: usize,
    pub propagation_edges: usize,
}

pub trait VizDataProvider: Send + Sync {
    fn replay_points(&self) -> Vec<ReplayPoint>;
    fn propagation_edges(&self) -> Vec<PropagationEdge>;
    fn feature_summary(&self) -> Vec<FeatureSummary>;
    fn recent_transactions(&self, limit: usize) -> Vec<TransactionSummary>;
}

#[derive(Clone)]
pub struct InMemoryVizProvider {
    storage: Arc<RwLock<InMemoryStorage>>,
    propagation: Arc<Vec<PropagationEdge>>,
    replay_stride: usize,
}

impl InMemoryVizProvider {
    pub fn new(
        storage: Arc<RwLock<InMemoryStorage>>,
        propagation: Arc<Vec<PropagationEdge>>,
        replay_stride: usize,
    ) -> Self {
        Self {
            storage,
            propagation,
            replay_stride: replay_stride.max(1),
        }
    }
}

impl VizDataProvider for InMemoryVizProvider {
    fn replay_points(&self) -> Vec<ReplayPoint> {
        let events = self
            .storage
            .read()
            .ok()
            .map(|storage| storage.list_events().to_vec())
            .unwrap_or_default();

        replay_frames(
            &events,
            ReplayMode::DeterministicEventReplay,
            self.replay_stride,
        )
        .into_iter()
        .map(|frame| ReplayPoint {
            seq_hi: frame.seq_hi,
            timestamp_unix_ms: frame.timestamp_unix_ms,
            pending_count: frame.pending.len() as u32,
        })
        .collect()
    }

    fn propagation_edges(&self) -> Vec<PropagationEdge> {
        (*self.propagation).clone()
    }

    fn feature_summary(&self) -> Vec<FeatureSummary> {
        self.storage
            .read()
            .ok()
            .map(|storage| {
                storage
                    .tx_features()
                    .iter()
                    .map(|feature| FeatureSummary {
                        protocol: feature.protocol.clone(),
                        category: feature.category.clone(),
                        count: 1,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn recent_transactions(&self, limit: usize) -> Vec<TransactionSummary> {
        let events = self
            .storage
            .read()
            .ok()
            .map(|storage| storage.list_events().to_vec())
            .unwrap_or_default();

        let mut out = Vec::new();
        let mut seen_hashes = HashSet::new();

        for event in events.iter().rev() {
            let decoded = match &event.payload {
                EventPayload::TxDecoded(decoded) => decoded,
                _ => continue,
            };
            if !seen_hashes.insert(decoded.hash) {
                continue;
            }
            out.push(TransactionSummary {
                hash: format_bytes(&decoded.hash),
                sender: format_bytes(&decoded.sender),
                nonce: decoded.nonce,
                tx_type: decoded.tx_type,
                seen_unix_ms: event.ingest_ts_unix_ms,
                source_id: event.source_id.0.clone(),
            });
            if out.len() >= limit {
                break;
            }
        }

        out
    }
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::OPTIONS])
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health))
        .route("/replay", get(replay))
        .route("/propagation", get(propagation))
        .route("/features", get(features))
        .route("/transactions", get(transactions))
        .route("/stream", get(stream))
        .layer(cors)
        .with_state(state)
}

pub fn default_state() -> AppState {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let live_rpc_config = LiveRpcConfig::from_env();
    if live_rpc_config.is_none() {
        if let Ok(mut store) = storage.write() {
            store.append_event(seed_decoded_event(1, 1, 1));
            store.append_event(seed_decoded_event(2, 2, 2));
            store.upsert_tx_features(TxFeaturesRecord {
                hash: hash_from_seq(1),
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                mev_score: 80,
            });
        }
    }

    let propagation = vec![
        PropagationEdge {
            source: "peer-a".to_owned(),
            destination: "peer-b".to_owned(),
            p50_delay_ms: 8,
            p99_delay_ms: 24,
        },
        PropagationEdge {
            source: "peer-a".to_owned(),
            destination: "peer-c".to_owned(),
            p50_delay_ms: 12,
            p99_delay_ms: 36,
        },
    ];
    if let Some(config) = live_rpc_config {
        start_live_rpc_feed(storage.clone(), config);
    } else {
        start_demo_event_feed(storage.clone());
    }

    AppState {
        provider: Arc::new(InMemoryVizProvider::new(storage, Arc::new(propagation), 1)),
        downsample_limit: 1_000,
    }
}

fn seed_decoded_event(seq_id: u64, hash_seed: u64, nonce: u64) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: current_unix_ms(),
        ingest_ts_mono_ns: seq_id.saturating_mul(1_000_000),
        source_id: common::SourceId::new("seed"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash_from_seq(hash_seed),
            tx_type: 2,
            sender: [9; 20],
            nonce,
        }),
    }
}

fn start_demo_event_feed(storage: Arc<RwLock<InMemoryStorage>>) {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => return,
    };

    handle.spawn(async move {
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        let mut seq_id = 3_u64;
        let mut nonce = 3_u64;

        loop {
            ticker.tick().await;
            let hash = hash_from_seq(seq_id);
            let protocol = if seq_id % 2 == 0 {
                "uniswap-v2"
            } else {
                "curve"
            };
            let category = if seq_id % 3 == 0 { "arb" } else { "swap" };
            let score = 50 + ((seq_id % 45) as u16);

            let mut store = match storage.write() {
                Ok(store) => store,
                Err(_) => break,
            };
            store.append_event(EventEnvelope {
                seq_id,
                ingest_ts_unix_ms: current_unix_ms(),
                ingest_ts_mono_ns: seq_id.saturating_mul(1_000_000),
                source_id: common::SourceId::new("demo-live"),
                payload: EventPayload::TxDecoded(TxDecoded {
                    hash,
                    tx_type: 2,
                    sender: [9; 20],
                    nonce,
                }),
            });
            store.upsert_tx_features(TxFeaturesRecord {
                hash,
                protocol: protocol.to_owned(),
                category: category.to_owned(),
                mev_score: score,
            });
            seq_id = seq_id.saturating_add(1);
            nonce = nonce.saturating_add(1);
        }
    });
}

fn hash_from_seq(seq: u64) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    hash[..8].copy_from_slice(&seq.to_be_bytes());
    hash
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn format_bytes(bytes: &[u8]) -> String {
    let mut out = String::from("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn downsample<T: Clone>(values: &[T], max_points: usize) -> Vec<T> {
    if values.len() <= max_points || max_points == 0 {
        return values.to_vec();
    }
    let step = ((values.len() as f64) / (max_points as f64)).ceil() as usize;
    values.iter().step_by(step.max(1)).cloned().collect()
}

async fn health() -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_owned(),
        }),
    )
}

async fn replay(State(state): State<AppState>) -> Json<Vec<ReplayPoint>> {
    let values = state.provider.replay_points();
    Json(downsample(&values, state.downsample_limit))
}

async fn propagation(State(state): State<AppState>) -> Json<Vec<PropagationEdge>> {
    let values = state.provider.propagation_edges();
    Json(downsample(&values, state.downsample_limit))
}

async fn features(State(state): State<AppState>) -> Json<Vec<FeatureSummary>> {
    let values = state.provider.feature_summary();
    Json(downsample(&values, state.downsample_limit))
}

#[derive(Clone, Debug, Default, Deserialize)]
struct TransactionsQuery {
    limit: Option<usize>,
}

async fn transactions(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<TransactionSummary>> {
    let limit = query.limit.unwrap_or(25).clamp(1, 200);
    Json(state.provider.recent_transactions(limit))
}

async fn stream(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let hello = StreamHello {
        event: "hello".to_owned(),
        message: "viz-api stream connected".to_owned(),
        replay_points: state.provider.replay_points().len(),
        propagation_edges: state.provider.propagation_edges().len(),
    };
    ws.on_upgrade(move |socket| handle_socket(socket, hello))
}

async fn handle_socket(mut socket: WebSocket, hello: StreamHello) {
    if let Ok(payload) = serde_json::to_string(&hello) {
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }

    while let Some(Ok(message)) = socket.recv().await {
        match message {
            Message::Close(_) => break,
            Message::Ping(data) => {
                if socket.send(Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use axum::http::header::{ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN};
    use axum::http::{HeaderValue, Method};
    use tower::util::ServiceExt;

    #[derive(Clone)]
    struct MockProvider;

    impl VizDataProvider for MockProvider {
        fn replay_points(&self) -> Vec<ReplayPoint> {
            vec![
                ReplayPoint {
                    seq_hi: 1,
                    timestamp_unix_ms: 100,
                    pending_count: 10,
                },
                ReplayPoint {
                    seq_hi: 2,
                    timestamp_unix_ms: 200,
                    pending_count: 20,
                },
                ReplayPoint {
                    seq_hi: 3,
                    timestamp_unix_ms: 300,
                    pending_count: 30,
                },
            ]
        }

        fn propagation_edges(&self) -> Vec<PropagationEdge> {
            vec![PropagationEdge {
                source: "a".to_owned(),
                destination: "b".to_owned(),
                p50_delay_ms: 1,
                p99_delay_ms: 2,
            }]
        }

        fn feature_summary(&self) -> Vec<FeatureSummary> {
            vec![FeatureSummary {
                protocol: "uni".to_owned(),
                category: "swap".to_owned(),
                count: 1,
            }]
        }

        fn recent_transactions(&self, limit: usize) -> Vec<TransactionSummary> {
            let values = vec![
                TransactionSummary {
                    hash: "0x01".to_owned(),
                    sender: "0xaa".to_owned(),
                    nonce: 7,
                    tx_type: 2,
                    seen_unix_ms: 1_700_000_000_000,
                    source_id: "mock".to_owned(),
                },
                TransactionSummary {
                    hash: "0x02".to_owned(),
                    sender: "0xbb".to_owned(),
                    nonce: 8,
                    tx_type: 2,
                    seen_unix_ms: 1_700_000_000_100,
                    source_id: "mock".to_owned(),
                },
            ];
            values.into_iter().take(limit).collect()
        }
    }

    fn test_state(limit: usize) -> AppState {
        AppState {
            provider: Arc::new(MockProvider),
            downsample_limit: limit,
        }
    }

    #[tokio::test]
    async fn health_route_returns_ok() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let payload: HealthResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.status, "ok");
    }

    #[tokio::test]
    async fn replay_route_applies_downsampling() {
        let app = build_router(test_state(2));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/replay")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<ReplayPoint> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0].seq_hi, 1);
        assert_eq!(payload[1].seq_hi, 3);
    }

    #[tokio::test]
    async fn default_state_replay_points_grow_over_time() {
        if std::env::var("VIZ_API_ETH_WS_URL").is_ok() {
            return;
        }
        let state = default_state();
        let before = state.provider.replay_points().len();

        tokio::time::sleep(std::time::Duration::from_millis(1_250)).await;

        let after = state.provider.replay_points().len();
        assert!(
            after > before,
            "expected replay points to grow, before={before}, after={after}"
        );
    }

    #[tokio::test]
    async fn replay_preflight_returns_cors_headers() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/replay")
                    .header("origin", "http://127.0.0.1:5173")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        let origin = response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN);
        assert!(origin.is_some(), "missing access-control-allow-origin");
        assert_eq!(origin, Some(&HeaderValue::from_static("*")));

        let methods = response.headers().get(ACCESS_CONTROL_ALLOW_METHODS);
        assert!(methods.is_some(), "missing access-control-allow-methods");
    }

    #[tokio::test]
    async fn transactions_route_respects_limit() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/transactions?limit=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<TransactionSummary> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].hash, "0x01");
    }
}
