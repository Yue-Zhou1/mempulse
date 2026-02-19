use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use replay::{replay_frames, ReplayMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::{EventStore, InMemoryStorage, TxFeaturesRecord};

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
}

#[derive(Clone)]
pub struct InMemoryVizProvider {
    storage: Arc<InMemoryStorage>,
    propagation: Arc<Vec<PropagationEdge>>,
    replay_stride: usize,
}

impl InMemoryVizProvider {
    pub fn new(
        storage: Arc<InMemoryStorage>,
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
        replay_frames(
            self.storage.list_events(),
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
            .tx_features()
            .iter()
            .map(|feature| FeatureSummary {
                protocol: feature.protocol.clone(),
                category: feature.category.clone(),
                count: 1,
            })
            .collect()
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/replay", get(replay))
        .route("/propagation", get(propagation))
        .route("/features", get(features))
        .route("/stream", get(stream))
        .with_state(state)
}

pub fn default_state() -> AppState {
    let mut storage = InMemoryStorage::default();

    storage.append_event(EventEnvelope {
        seq_id: 1,
        ingest_ts_unix_ms: 1_700_000_000_000,
        ingest_ts_mono_ns: 10,
        source_id: common::SourceId::new("seed"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: [1; 32],
            tx_type: 2,
            sender: [9; 20],
            nonce: 1,
        }),
    });
    storage.append_event(EventEnvelope {
        seq_id: 2,
        ingest_ts_unix_ms: 1_700_000_012_000,
        ingest_ts_mono_ns: 20,
        source_id: common::SourceId::new("seed"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: [2; 32],
            tx_type: 2,
            sender: [9; 20],
            nonce: 2,
        }),
    });
    storage.upsert_tx_features(TxFeaturesRecord {
        hash: [1; 32],
        protocol: "uniswap-v2".to_owned(),
        category: "swap".to_owned(),
        mev_score: 80,
    });

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

    AppState {
        provider: Arc::new(InMemoryVizProvider::new(
            Arc::new(storage),
            Arc::new(propagation),
            1,
        )),
        downsample_limit: 1_000,
    }
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
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
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
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
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
            .oneshot(Request::builder().uri("/replay").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<ReplayPoint> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0].seq_hi, 1);
        assert_eq!(payload[1].seq_hi, 3);
    }
}
