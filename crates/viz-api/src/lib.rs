pub mod auth;
pub mod live_rpc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{middleware, response::Response};
use axum::routing::get;
use axum::{Json, Router};
use auth::{ApiAuthConfig, ApiRateLimiter};
use builder::RelayDryRunStatus;
use common::{AlertDecisions, AlertThresholdConfig, MetricSnapshot, evaluate_alerts};
use live_rpc::{LiveRpcConfig, start_live_rpc_feed};
use replay::{ReplayMode, replay_frames};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::{Arc, RwLock};
use storage::{
    ClickHouseBatchSink, ClickHouseHttpSink, EventStore, InMemoryStorage, NoopClickHouseSink,
    StorageWriterConfig, spawn_single_writer,
};
use tower_http::cors::{Any, CorsLayer};

#[cfg(test)]
use event_log::{EventEnvelope, EventPayload, TxDecoded};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct AppState {
    pub provider: Arc<dyn VizDataProvider>,
    pub downsample_limit: usize,
    pub relay_dry_run_status: Arc<RwLock<RelayDryRunStatus>>,
    pub alert_thresholds: AlertThresholdConfig,
    pub api_auth: ApiAuthConfig,
    pub api_rate_limiter: ApiRateLimiter,
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
pub struct FeatureDetail {
    pub hash: String,
    pub protocol: String,
    pub category: String,
    pub mev_score: u16,
    pub urgency_score: u16,
    pub method_selector: Option<String>,
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
pub struct TransactionDetail {
    pub hash: String,
    pub peer: String,
    pub first_seen_unix_ms: i64,
    pub seen_count: u32,
    pub tx_type: Option<u8>,
    pub sender: Option<String>,
    pub to: Option<String>,
    pub chain_id: Option<u64>,
    pub nonce: Option<u64>,
    pub value_wei: Option<u128>,
    pub gas_limit: Option<u64>,
    pub gas_price_wei: Option<u128>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub max_fee_per_blob_gas_wei: Option<u128>,
    pub calldata_len: Option<u32>,
    pub raw_tx_len: Option<usize>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IngestSourceMode {
    Rpc,
    P2p,
    Hybrid,
}

impl IngestSourceMode {
    fn as_str(self) -> &'static str {
        match self {
            IngestSourceMode::Rpc => "rpc",
            IngestSourceMode::P2p => "p2p",
            IngestSourceMode::Hybrid => "hybrid",
        }
    }
}

fn resolve_ingest_source_mode(env_override: Option<&str>) -> IngestSourceMode {
    match env_override.map(str::trim).map(str::to_ascii_lowercase) {
        Some(mode) if mode == "p2p" => IngestSourceMode::P2p,
        Some(mode) if mode == "hybrid" => IngestSourceMode::Hybrid,
        _ => IngestSourceMode::Rpc,
    }
}

pub trait VizDataProvider: Send + Sync {
    fn replay_points(&self) -> Vec<ReplayPoint>;
    fn propagation_edges(&self) -> Vec<PropagationEdge>;
    fn feature_summary(&self) -> Vec<FeatureSummary>;
    fn feature_details(&self, limit: usize) -> Vec<FeatureDetail>;
    fn recent_transactions(&self, limit: usize) -> Vec<TransactionSummary>;
    fn transaction_details(&self, limit: usize) -> Vec<TransactionDetail>;
    fn transaction_detail_by_hash(&self, hash: &str) -> Option<TransactionDetail>;
    fn metric_snapshot(&self) -> MetricSnapshot;
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
            .map(|storage| storage.list_events())
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

    fn feature_details(&self, limit: usize) -> Vec<FeatureDetail> {
        self.storage
            .read()
            .ok()
            .map(|storage| {
                let mut emitted = std::collections::HashSet::new();
                let mut out = Vec::new();

                for feature in storage.tx_features().iter().rev() {
                    if !emitted.insert(feature.hash) {
                        continue;
                    }
                    out.push(FeatureDetail {
                        hash: format_bytes(&feature.hash),
                        protocol: feature.protocol.clone(),
                        category: feature.category.clone(),
                        mev_score: feature.mev_score,
                        urgency_score: feature.urgency_score,
                        method_selector: format_method_selector(feature.method_selector),
                    });
                    if out.len() >= limit {
                        break;
                    }
                }

                out
            })
            .unwrap_or_default()
    }

    fn recent_transactions(&self, limit: usize) -> Vec<TransactionSummary> {
        self.storage
            .read()
            .ok()
            .map(|storage| {
                storage
                    .recent_transactions(limit)
                    .into_iter()
                    .map(|tx| TransactionSummary {
                        hash: format_bytes(&tx.hash),
                        sender: format_bytes(&tx.sender),
                        nonce: tx.nonce,
                        tx_type: tx.tx_type,
                        seen_unix_ms: tx.seen_unix_ms,
                        source_id: tx.source_id,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn transaction_details(&self, limit: usize) -> Vec<TransactionDetail> {
        self.storage
            .read()
            .ok()
            .map(|storage| {
                let mut full_by_hash = std::collections::HashMap::new();
                for row in storage.tx_full() {
                    full_by_hash.insert(row.hash, row);
                }

                let mut emitted = std::collections::HashSet::new();
                let mut out = Vec::new();

                for seen in storage.tx_seen().iter().rev() {
                    if !emitted.insert(seen.hash) {
                        continue;
                    }
                    let full = full_by_hash.get(&seen.hash).copied();
                    out.push(TransactionDetail {
                        hash: format_bytes(&seen.hash),
                        peer: seen.peer.clone(),
                        first_seen_unix_ms: seen.first_seen_unix_ms,
                        seen_count: seen.seen_count,
                        tx_type: full.map(|row| row.tx_type),
                        sender: full.map(|row| format_bytes(&row.sender)),
                        to: full.and_then(|row| row.to.as_ref().map(|to| format_bytes(to))),
                        chain_id: full.and_then(|row| row.chain_id),
                        nonce: full.map(|row| row.nonce),
                        value_wei: full.and_then(|row| row.value_wei),
                        gas_limit: full.and_then(|row| row.gas_limit),
                        gas_price_wei: full.and_then(|row| row.gas_price_wei),
                        max_fee_per_gas_wei: full.and_then(|row| row.max_fee_per_gas_wei),
                        max_priority_fee_per_gas_wei: full
                            .and_then(|row| row.max_priority_fee_per_gas_wei),
                        max_fee_per_blob_gas_wei: full.and_then(|row| row.max_fee_per_blob_gas_wei),
                        calldata_len: full.and_then(|row| row.calldata_len),
                        raw_tx_len: full.map(|row| row.raw_tx.len()),
                    });
                    if out.len() >= limit {
                        break;
                    }
                }

                out
            })
            .unwrap_or_default()
    }

    fn transaction_detail_by_hash(&self, hash: &str) -> Option<TransactionDetail> {
        let hash = parse_fixed_hex::<32>(hash)?;
        self.storage.read().ok().and_then(|storage| {
            let seen = storage.tx_seen().iter().rev().find(|seen| seen.hash == hash)?;
            let full = storage.tx_full().iter().rev().find(|row| row.hash == hash);
            Some(TransactionDetail {
                hash: format_bytes(&seen.hash),
                peer: seen.peer.clone(),
                first_seen_unix_ms: seen.first_seen_unix_ms,
                seen_count: seen.seen_count,
                tx_type: full.map(|row| row.tx_type),
                sender: full.map(|row| format_bytes(&row.sender)),
                to: full.and_then(|row| row.to.as_ref().map(|to| format_bytes(to))),
                chain_id: full.and_then(|row| row.chain_id),
                nonce: full.map(|row| row.nonce),
                value_wei: full.and_then(|row| row.value_wei),
                gas_limit: full.and_then(|row| row.gas_limit),
                gas_price_wei: full.and_then(|row| row.gas_price_wei),
                max_fee_per_gas_wei: full.and_then(|row| row.max_fee_per_gas_wei),
                max_priority_fee_per_gas_wei: full.and_then(|row| row.max_priority_fee_per_gas_wei),
                max_fee_per_blob_gas_wei: full.and_then(|row| row.max_fee_per_blob_gas_wei),
                calldata_len: full.and_then(|row| row.calldata_len),
                raw_tx_len: full.map(|row| row.raw_tx.len()),
            })
        })
    }

    fn metric_snapshot(&self) -> MetricSnapshot {
        let (tx_seen_len, tx_full_len) = self
            .storage
            .read()
            .ok()
            .map(|storage| (storage.tx_seen().len() as u64, storage.tx_full().len() as u64))
            .unwrap_or((0, 0));
        let queue_depth_capacity = 10_000_u64;

        MetricSnapshot {
            peer_disconnects_total: 0,
            ingest_lag_ms: 0,
            tx_decode_fail_total: tx_seen_len.saturating_sub(tx_full_len),
            tx_decode_total: tx_seen_len.max(1),
            tx_per_sec_current: tx_seen_len,
            tx_per_sec_baseline: tx_seen_len.max(1),
            storage_write_latency_ms: 0,
            clock_skew_ms: 0,
            queue_depth_current: tx_seen_len.min(queue_depth_capacity),
            queue_depth_capacity,
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::OPTIONS])
        .allow_headers(Any);

    let protected = Router::new()
        .route("/replay", get(replay))
        .route("/propagation", get(propagation))
        .route("/metrics/snapshot", get(metrics_snapshot))
        .route("/alerts/evaluate", get(alerts_evaluate))
        .route("/features", get(features))
        .route("/features/recent", get(features_recent))
        .route("/transactions", get(transactions))
        .route("/transactions/all", get(transactions_all))
        .route("/transactions/{hash}", get(transaction_by_hash))
        .route("/relay/dry-run/status", get(relay_dry_run_status))
        .route("/stream", get(stream))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .layer(cors)
        .with_state(state)
}

pub fn default_state() -> AppState {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let sink: Arc<dyn ClickHouseBatchSink> = match ClickHouseHttpSink::from_env() {
        Ok(Some(sink)) => Arc::new(sink),
        Ok(None) => Arc::new(NoopClickHouseSink),
        Err(err) => {
            tracing::warn!(error = %err, "failed to initialize clickhouse sink, falling back to noop sink");
            Arc::new(NoopClickHouseSink)
        }
    };
    let writer = spawn_single_writer(storage.clone(), sink, StorageWriterConfig::default());
    let live_rpc_config = match LiveRpcConfig::from_env() {
        Ok(config) => config,
        Err(err) => {
            tracing::warn!(error = %err, "failed to parse live rpc env overrides; using defaults");
            LiveRpcConfig::default()
        }
    };
    let ingest_mode =
        resolve_ingest_source_mode(env::var("VIZ_API_INGEST_MODE").ok().as_deref());

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
    match ingest_mode {
        IngestSourceMode::Rpc => {
            tracing::info!(ingest_mode = ingest_mode.as_str(), "starting rpc ingest mode");
            start_live_rpc_feed(storage.clone(), writer, live_rpc_config);
        }
        IngestSourceMode::P2p => {
            tracing::info!(
                ingest_mode = ingest_mode.as_str(),
                "starting p2p ingest mode (external devp2p runtime expected)"
            );
        }
        IngestSourceMode::Hybrid => {
            tracing::info!(
                ingest_mode = ingest_mode.as_str(),
                "starting hybrid ingest mode (rpc live feed + p2p runtime)"
            );
            start_live_rpc_feed(storage.clone(), writer, live_rpc_config);
        }
    }

    let api_auth = ApiAuthConfig::from_env();
    if api_auth.enabled && api_auth.api_keys.is_empty() {
        tracing::warn!("api auth enabled but no API keys configured; all protected routes will return unauthorized");
    }
    let api_rate_limiter = ApiRateLimiter::new(api_auth.requests_per_minute);

    AppState {
        provider: Arc::new(InMemoryVizProvider::new(storage, Arc::new(propagation), 1)),
        downsample_limit: 1_000,
        relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
        alert_thresholds: AlertThresholdConfig::default(),
        api_auth,
        api_rate_limiter,
    }
}

#[cfg(test)]
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
            chain_id: Some(1),
            to: None,
            value_wei: None,
            gas_limit: None,
            gas_price_wei: None,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            max_fee_per_blob_gas_wei: None,
            calldata_len: None,
        }),
    }
}

#[cfg(test)]
fn hash_from_seq(seq: u64) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    hash[..8].copy_from_slice(&seq.to_be_bytes());
    hash
}

#[cfg(test)]
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

fn format_method_selector(method_selector: Option<[u8; 4]>) -> Option<String> {
    method_selector.map(|selector| {
        format!(
            "0x{:02x}{:02x}{:02x}{:02x}",
            selector[0], selector[1], selector[2], selector[3]
        )
    })
}

fn parse_fixed_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = parse_hex_bytes(value)?;
    if bytes.len() != N {
        return None;
    }
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn parse_hex_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    if trimmed.len() % 2 != 0 {
        return None;
    }
    (0..trimmed.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&trimmed[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()
}

pub fn downsample<T: Clone>(values: &[T], max_points: usize) -> Vec<T> {
    if values.len() <= max_points || max_points == 0 {
        return values.to_vec();
    }
    let step = ((values.len() as f64) / (max_points as f64)).ceil() as usize;
    values.iter().step_by(step.max(1)).cloned().collect()
}

async fn require_api_key(
    State(state): State<AppState>,
    request: Request,
    next: middleware::Next,
) -> Response {
    if !state.api_auth.enabled {
        return next.run(request).await;
    }

    let maybe_key = request
        .headers()
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let api_key = match maybe_key {
        Some(api_key) if state.api_auth.validates_key(api_key) => api_key,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };

    if !state.api_rate_limiter.allow(api_key) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    next.run(request).await
}

async fn health() -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_owned(),
        }),
    )
}

async fn metrics_snapshot(State(state): State<AppState>) -> Json<MetricSnapshot> {
    Json(state.provider.metric_snapshot())
}

async fn alerts_evaluate(State(state): State<AppState>) -> Json<AlertDecisions> {
    let snapshot = state.provider.metric_snapshot();
    Json(evaluate_alerts(&snapshot, &state.alert_thresholds))
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

async fn features_recent(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<FeatureDetail>> {
    let limit = query.limit.unwrap_or(100).clamp(1, 5_000);
    Json(state.provider.feature_details(limit))
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

async fn transactions_all(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<TransactionDetail>> {
    let limit = query.limit.unwrap_or(1_000).clamp(1, 5_000);
    Json(state.provider.transaction_details(limit))
}

async fn transaction_by_hash(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Json<TransactionDetail>, StatusCode> {
    state
        .provider
        .transaction_detail_by_hash(&hash)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn relay_dry_run_status(State(state): State<AppState>) -> Json<RelayDryRunStatus> {
    let status = state
        .relay_dry_run_status
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    Json(status)
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

        fn feature_details(&self, limit: usize) -> Vec<FeatureDetail> {
            let values = vec![
                FeatureDetail {
                    hash: "0x01".to_owned(),
                    protocol: "uniswap-v2".to_owned(),
                    category: "swap".to_owned(),
                    mev_score: 72,
                    urgency_score: 18,
                    method_selector: Some("0x38ed1739".to_owned()),
                },
                FeatureDetail {
                    hash: "0x02".to_owned(),
                    protocol: "erc20".to_owned(),
                    category: "transfer".to_owned(),
                    mev_score: 18,
                    urgency_score: 7,
                    method_selector: Some("0xa9059cbb".to_owned()),
                },
            ];
            values.into_iter().take(limit).collect()
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

        fn transaction_details(&self, limit: usize) -> Vec<TransactionDetail> {
            let values = vec![
                TransactionDetail {
                    hash: "0x01".to_owned(),
                    peer: "rpc-ws".to_owned(),
                    first_seen_unix_ms: 1_700_000_000_000,
                    seen_count: 1,
                    tx_type: Some(2),
                    sender: Some("0xaa".to_owned()),
                    to: Some("0xcc".to_owned()),
                    chain_id: Some(1),
                    nonce: Some(7),
                    value_wei: Some(1_000_000_000_000_000_000),
                    gas_limit: Some(180_000),
                    gas_price_wei: Some(45_000_000_000),
                    max_fee_per_gas_wei: Some(60_000_000_000),
                    max_priority_fee_per_gas_wei: Some(2_000_000_000),
                    max_fee_per_blob_gas_wei: Some(3),
                    calldata_len: Some(120),
                    raw_tx_len: Some(120),
                },
                TransactionDetail {
                    hash: "0x02".to_owned(),
                    peer: "rpc-ws".to_owned(),
                    first_seen_unix_ms: 1_700_000_000_100,
                    seen_count: 1,
                    tx_type: None,
                    sender: None,
                    to: None,
                    chain_id: None,
                    nonce: None,
                    value_wei: None,
                    gas_limit: None,
                    gas_price_wei: None,
                    max_fee_per_gas_wei: None,
                    max_priority_fee_per_gas_wei: None,
                    max_fee_per_blob_gas_wei: None,
                    calldata_len: None,
                    raw_tx_len: None,
                },
            ];
            values.into_iter().take(limit).collect()
        }

        fn transaction_detail_by_hash(&self, hash: &str) -> Option<TransactionDetail> {
            self.transaction_details(100)
                .into_iter()
                .find(|row| row.hash == hash)
        }

        fn metric_snapshot(&self) -> MetricSnapshot {
            MetricSnapshot {
                peer_disconnects_total: 0,
                ingest_lag_ms: 700,
                tx_decode_fail_total: 25,
                tx_decode_total: 1_000,
                tx_per_sec_current: 450,
                tx_per_sec_baseline: 1_000,
                storage_write_latency_ms: 200,
                clock_skew_ms: 50,
                queue_depth_current: 9_600,
                queue_depth_capacity: 10_000,
            }
        }
    }

    fn test_state(limit: usize) -> AppState {
        let api_auth = ApiAuthConfig::default();
        AppState {
            provider: Arc::new(MockProvider),
            downsample_limit: limit,
            relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
            alert_thresholds: AlertThresholdConfig::default(),
            api_rate_limiter: ApiRateLimiter::new(api_auth.requests_per_minute),
            api_auth,
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
    async fn default_state_initializes_live_rpc_without_env() {
        let state = default_state();
        let _ = state.provider.replay_points();
        assert_eq!(state.downsample_limit, 1_000);
    }

    #[test]
    fn resolve_ingest_source_mode_defaults_to_rpc() {
        assert_eq!(resolve_ingest_source_mode(None), IngestSourceMode::Rpc);
        assert_eq!(
            resolve_ingest_source_mode(Some("unexpected")),
            IngestSourceMode::Rpc
        );
    }

    #[test]
    fn resolve_ingest_source_mode_accepts_p2p_and_hybrid() {
        assert_eq!(resolve_ingest_source_mode(Some("p2p")), IngestSourceMode::P2p);
        assert_eq!(
            resolve_ingest_source_mode(Some("hybrid")),
            IngestSourceMode::Hybrid
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
    async fn metrics_snapshot_route_returns_provider_snapshot() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: MetricSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.ingest_lag_ms, 700);
        assert_eq!(payload.queue_depth_current, 9_600);
    }

    #[tokio::test]
    async fn alerts_evaluate_route_returns_decisions() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/alerts/evaluate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: AlertDecisions = serde_json::from_slice(&body).unwrap();
        assert!(payload.ingest_lag);
        assert!(payload.decode_failure);
        assert!(payload.coverage_collapse);
        assert!(payload.queue_saturation);
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

    #[tokio::test]
    async fn transactions_all_route_returns_detail_rows() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/transactions/all?limit=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<TransactionDetail> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0].hash, "0x01");
        assert_eq!(payload[1].hash, "0x02");
    }

    #[tokio::test]
    async fn transaction_by_hash_route_returns_detail_row() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/transactions/0x01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: TransactionDetail = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.hash, "0x01");
        assert_eq!(payload.sender.as_deref(), Some("0xaa"));
    }

    #[tokio::test]
    async fn transaction_by_hash_route_returns_404_for_unknown_hash() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/transactions/0xdeadbeef")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn features_recent_route_returns_rows() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/features/recent?limit=2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<FeatureDetail> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0].protocol, "uniswap-v2");
        assert_eq!(payload[0].mev_score, 72);
    }

    #[tokio::test]
    async fn relay_dry_run_status_route_returns_default_state() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/relay/dry-run/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: RelayDryRunStatus = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.total_submissions, 0);
        assert!(payload.latest.is_none());
    }

    #[test]
    fn in_memory_provider_recent_transactions_are_latest_first_and_deduped() {
        let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
        {
            let mut guard = storage.write().expect("write lock");
            guard.append_event(seed_decoded_event(1, 1, 1));
            guard.append_event(seed_decoded_event(2, 2, 2));
            // Same hash as seq 1, newer nonce should replace previous recent-tx record.
            guard.append_event(seed_decoded_event(3, 1, 3));
        }

        let provider = InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1);
        let recent = provider.recent_transactions(10);

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].nonce, 3);
        assert_eq!(recent[1].nonce, 2);
    }

    #[test]
    fn in_memory_provider_transaction_detail_by_hash_finds_row() {
        let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
        let hash = hash_from_seq(99);
        {
            let mut guard = storage.write().expect("write lock");
            guard.upsert_tx_seen(storage::TxSeenRecord {
                hash,
                peer: "rpc-ws".to_owned(),
                first_seen_unix_ms: 1_700_000_000_000,
                first_seen_mono_ns: 1_700_000_000_000_000_000,
                seen_count: 2,
            });
            guard.upsert_tx_full(storage::TxFullRecord {
                hash,
                tx_type: 2,
                sender: [7_u8; 20],
                nonce: 42,
                to: None,
                chain_id: Some(1),
                value_wei: None,
                gas_limit: None,
                gas_price_wei: None,
                max_fee_per_gas_wei: None,
                max_priority_fee_per_gas_wei: None,
                max_fee_per_blob_gas_wei: None,
                calldata_len: Some(3),
                raw_tx: vec![0xaa, 0xbb, 0xcc],
            });
        }

        let provider = InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1);
        let detail = provider
            .transaction_detail_by_hash(&format_bytes(&hash))
            .expect("detail row");

        assert_eq!(detail.peer, "rpc-ws");
        assert_eq!(detail.nonce, Some(42));
        assert_eq!(detail.raw_tx_len, Some(3));
    }
}
