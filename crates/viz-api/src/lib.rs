pub mod auth;
pub mod live_rpc;

use auth::{ApiAuthConfig, ApiRateLimiter};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use axum::{middleware, response::Response};
use builder::{RelayDryRunResult, RelayDryRunStatus};
use common::{AlertDecisions, AlertThresholdConfig, MetricSnapshot, evaluate_alerts};
use event_log::{EventEnvelope, EventPayload};
use live_rpc::{
    LiveRpcChainStatus, LiveRpcConfig, LiveRpcDropMetricsSnapshot, live_rpc_chain_status_snapshot,
    live_rpc_drop_metrics_snapshot,
};
use replay::{
    ReplayMode, TxLifecycleStatus, current_lifecycle, replay_diff_summary, replay_frames,
    replay_from_checkpoint,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use storage::{
    ClickHouseBatchSink, ClickHouseHttpSink, EventStore, InMemoryStorage, MarketStatsSnapshot,
    NoopClickHouseSink, StorageWriteHandle, StorageWriterConfig, spawn_single_writer,
};
use tokio::time::MissedTickBehavior;
use tower_http::cors::{Any, CorsLayer};

#[cfg(test)]
use builder::RelayAttemptTrace;
#[cfg(test)]
use event_log::{TxConfirmed, TxDecoded, TxFetched, TxReorged, TxSeen};
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
    pub live_rpc_chain_status_provider: Arc<dyn Fn() -> Vec<LiveRpcChainStatus> + Send + Sync>,
    pub live_rpc_drop_metrics_provider: Arc<dyn Fn() -> LiveRpcDropMetricsSnapshot + Send + Sync>,
}

#[derive(Clone)]
pub struct RuntimeBootstrap {
    pub storage: Arc<RwLock<InMemoryStorage>>,
    pub writer: StorageWriteHandle,
    pub live_rpc_config: LiveRpcConfig,
    pub ingest_mode: IngestSourceMode,
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
    pub chain_id: Option<u64>,
    pub mev_score: u16,
    pub urgency_score: u16,
    pub method_selector: Option<String>,
    pub feature_engine_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpportunityDetail {
    pub tx_hash: String,
    pub status: String,
    pub strategy: String,
    pub score: u32,
    pub protocol: String,
    pub category: String,
    pub chain_id: Option<u64>,
    pub feature_engine_version: String,
    pub scorer_version: String,
    pub strategy_version: String,
    pub reasons: Vec<String>,
    pub detected_unix_ms: i64,
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
    pub lifecycle_status: Option<String>,
    pub lifecycle_reason: Option<String>,
    pub lifecycle_updated_unix_ms: Option<i64>,
    pub protocol: Option<String>,
    pub category: Option<String>,
    pub mev_score: Option<u16>,
    pub urgency_score: Option<u16>,
    pub feature_engine_version: Option<String>,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub replay: Vec<ReplayPoint>,
    pub propagation: Vec<PropagationEdge>,
    pub opportunities: Vec<OpportunityDetail>,
    pub feature_summary: Vec<FeatureSummary>,
    pub feature_details: Vec<FeatureDetail>,
    pub transactions: Vec<TransactionSummary>,
    pub chain_ingest_status: Vec<LiveRpcChainStatus>,
    pub market_stats: MarketStats,
    pub latest_seq_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketStats {
    pub total_signal_volume: u64,
    pub total_tx_count: u64,
    pub low_risk_count: u64,
    pub medium_risk_count: u64,
    pub high_risk_count: u64,
    pub success_rate_bps: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayRangeDiffSummary {
    pub from_pending_count: usize,
    pub to_pending_count: usize,
    pub added_pending_count: usize,
    pub removed_pending_count: usize,
    pub added_pending: Vec<String>,
    pub removed_pending: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayRangeResponse {
    pub from_seq_id: u64,
    pub to_seq_id: u64,
    pub from_checkpoint_hash: String,
    pub to_checkpoint_hash: String,
    pub summary: ReplayRangeDiffSummary,
    pub frames: Vec<ReplayPoint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BundleDetail {
    pub id: String,
    pub relay_url: String,
    pub accepted: bool,
    pub final_state: String,
    pub attempt_count: usize,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimDetail {
    pub id: String,
    pub bundle_id: String,
    pub status: String,
    pub relay_url: String,
    pub attempt_count: usize,
    pub accepted: bool,
    pub fail_category: Option<String>,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestSourceMode {
    Rpc,
    P2p,
    Hybrid,
}

impl IngestSourceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            IngestSourceMode::Rpc => "rpc",
            IngestSourceMode::P2p => "p2p",
            IngestSourceMode::Hybrid => "hybrid",
        }
    }
}

pub fn resolve_ingest_source_mode(env_override: Option<&str>) -> IngestSourceMode {
    match env_override.map(str::trim).map(str::to_ascii_lowercase) {
        Some(mode) if mode == "p2p" => IngestSourceMode::P2p,
        Some(mode) if mode == "hybrid" => IngestSourceMode::Hybrid,
        _ => IngestSourceMode::Rpc,
    }
}

fn market_success_rate_bps(total_tx_count: u64, high_risk_count: u64) -> u16 {
    if total_tx_count == 0 {
        return 10_000;
    }
    let successful = total_tx_count.saturating_sub(high_risk_count);
    let bps = successful
        .saturating_mul(10_000)
        .saturating_div(total_tx_count);
    bps.min(10_000) as u16
}

fn map_market_stats(snapshot: MarketStatsSnapshot) -> MarketStats {
    MarketStats {
        total_signal_volume: snapshot.total_signal_volume,
        total_tx_count: snapshot.total_tx_count,
        low_risk_count: snapshot.low_risk_count,
        medium_risk_count: snapshot.medium_risk_count,
        high_risk_count: snapshot.high_risk_count,
        success_rate_bps: market_success_rate_bps(
            snapshot.total_tx_count,
            snapshot.high_risk_count,
        ),
    }
}

pub trait VizDataProvider: Send + Sync {
    fn events(&self, after_seq_id: u64, event_types: &[String], limit: usize)
    -> Vec<EventEnvelope>;
    fn replay_points(&self) -> Vec<ReplayPoint>;
    fn propagation_edges(&self) -> Vec<PropagationEdge>;
    fn feature_summary(&self) -> Vec<FeatureSummary>;
    fn feature_details(&self, limit: usize) -> Vec<FeatureDetail>;
    fn opportunities(&self, limit: usize, min_score: u32) -> Vec<OpportunityDetail>;
    fn recent_transactions(&self, limit: usize) -> Vec<TransactionSummary>;
    fn transaction_details(&self, limit: usize) -> Vec<TransactionDetail>;
    fn transaction_detail_by_hash(&self, hash: &str) -> Option<TransactionDetail>;
    fn market_stats(&self) -> MarketStats;
    fn metric_snapshot(&self) -> MetricSnapshot;
}

#[derive(Clone)]
pub struct InMemoryVizProvider {
    storage: Arc<RwLock<InMemoryStorage>>,
    propagation: Arc<Vec<PropagationEdge>>,
    replay_stride: usize,
    dashboard_cache: Arc<RwLock<DashboardReadCache>>,
}

#[derive(Clone, Debug, Default)]
struct DashboardReadCache {
    revision: Option<u64>,
    refreshes: u64,
    replay_points: Vec<ReplayPoint>,
    feature_summary: Vec<FeatureSummary>,
    feature_details: Vec<FeatureDetail>,
    opportunities: Vec<OpportunityDetail>,
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
            dashboard_cache: Arc::new(RwLock::new(DashboardReadCache::default())),
        }
    }

    pub fn dashboard_cache_refreshes(&self) -> u64 {
        self.dashboard_cache
            .read()
            .map(|cache| cache.refreshes)
            .unwrap_or(0)
    }

    fn dashboard_cache_snapshot(&self) -> DashboardReadCache {
        let storage = match self.storage.read() {
            Ok(storage) => storage,
            Err(_) => return DashboardReadCache::default(),
        };
        let revision = storage.read_model_revision();

        let mut cache = match self.dashboard_cache.write() {
            Ok(cache) => cache,
            Err(_) => return DashboardReadCache::default(),
        };
        if cache.revision != Some(revision) {
            cache.revision = Some(revision);
            cache.replay_points = build_replay_points(&storage, self.replay_stride);
            cache.feature_summary = build_feature_summary(&storage);
            cache.feature_details = build_feature_details(&storage);
            cache.opportunities = build_opportunities(&storage);
            cache.refreshes = cache.refreshes.saturating_add(1);
        }
        cache.clone()
    }
}

fn build_replay_points(storage: &InMemoryStorage, replay_stride: usize) -> Vec<ReplayPoint> {
    replay_frames(
        &storage.list_events(),
        ReplayMode::DeterministicEventReplay,
        replay_stride,
    )
    .into_iter()
    .map(|frame| ReplayPoint {
        seq_hi: frame.seq_hi,
        timestamp_unix_ms: frame.timestamp_unix_ms,
        pending_count: frame.pending.len() as u32,
    })
    .collect()
}

fn build_feature_summary(storage: &InMemoryStorage) -> Vec<FeatureSummary> {
    let mut counts = std::collections::BTreeMap::<(String, String), u64>::new();
    for feature in storage.tx_features() {
        *counts
            .entry((feature.protocol.clone(), feature.category.clone()))
            .or_insert(0) += 1;
    }

    let mut out = counts
        .into_iter()
        .map(|((protocol, category), count)| FeatureSummary {
            protocol,
            category,
            count,
        })
        .collect::<Vec<_>>();
    out.sort_unstable_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.protocol.cmp(&right.protocol))
            .then_with(|| left.category.cmp(&right.category))
    });
    out
}

fn build_feature_details(storage: &InMemoryStorage) -> Vec<FeatureDetail> {
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
            chain_id: feature.chain_id,
            mev_score: feature.mev_score,
            urgency_score: feature.urgency_score,
            method_selector: format_method_selector(feature.method_selector),
            feature_engine_version: feature.feature_engine_version.clone(),
        });
    }

    out
}

fn build_opportunities(storage: &InMemoryStorage) -> Vec<OpportunityDetail> {
    storage
        .opportunities()
        .into_iter()
        .map(|row| OpportunityDetail {
            tx_hash: format_bytes(&row.tx_hash),
            status: "detected".to_owned(),
            strategy: row.strategy,
            score: row.score,
            protocol: row.protocol,
            category: row.category,
            chain_id: row.chain_id,
            feature_engine_version: row.feature_engine_version,
            scorer_version: row.scorer_version,
            strategy_version: row.strategy_version,
            reasons: row.reasons,
            detected_unix_ms: row.detected_unix_ms,
        })
        .collect()
}

impl VizDataProvider for InMemoryVizProvider {
    fn events(
        &self,
        after_seq_id: u64,
        event_types: &[String],
        limit: usize,
    ) -> Vec<EventEnvelope> {
        self.storage
            .read()
            .ok()
            .map(|storage| {
                storage
                    .scan_events(after_seq_id, limit.saturating_mul(2).max(limit))
                    .into_iter()
                    .filter(|event| {
                        event_types.is_empty()
                            || event_types.iter().any(|kind| {
                                event_payload_type(&event.payload).eq_ignore_ascii_case(kind)
                            })
                    })
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn replay_points(&self) -> Vec<ReplayPoint> {
        self.dashboard_cache_snapshot().replay_points
    }

    fn propagation_edges(&self) -> Vec<PropagationEdge> {
        let mut edges = (*self.propagation).clone();
        edges.sort_unstable_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.destination.cmp(&right.destination))
                .then_with(|| left.p50_delay_ms.cmp(&right.p50_delay_ms))
                .then_with(|| left.p99_delay_ms.cmp(&right.p99_delay_ms))
        });
        edges
    }

    fn feature_summary(&self) -> Vec<FeatureSummary> {
        self.dashboard_cache_snapshot().feature_summary
    }

    fn feature_details(&self, limit: usize) -> Vec<FeatureDetail> {
        self.dashboard_cache_snapshot()
            .feature_details
            .into_iter()
            .take(limit.max(1))
            .collect()
    }

    fn opportunities(&self, limit: usize, min_score: u32) -> Vec<OpportunityDetail> {
        self.dashboard_cache_snapshot()
            .opportunities
            .into_iter()
            .filter(|row| row.score >= min_score)
            .take(limit)
            .collect()
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

    fn market_stats(&self) -> MarketStats {
        self.storage
            .read()
            .ok()
            .map(|storage| map_market_stats(storage.market_stats_snapshot()))
            .unwrap_or_else(|| map_market_stats(MarketStatsSnapshot::default()))
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
                let mut feature_by_hash = std::collections::HashMap::new();
                for row in storage.tx_features() {
                    feature_by_hash.insert(row.hash, row);
                }
                let mut lifecycle_by_hash = std::collections::HashMap::new();
                for row in storage.tx_lifecycle() {
                    lifecycle_by_hash.insert(row.hash, row);
                }

                let mut emitted = std::collections::HashSet::new();
                let mut out = Vec::new();

                for seen in storage.tx_seen().iter().rev() {
                    if !emitted.insert(seen.hash) {
                        continue;
                    }
                    let full = full_by_hash.get(&seen.hash).copied();
                    let feature = feature_by_hash.get(&seen.hash).copied();
                    let lifecycle = lifecycle_by_hash.get(&seen.hash).copied();
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
                        lifecycle_status: lifecycle.map(|row| row.status.clone()),
                        lifecycle_reason: lifecycle.and_then(|row| row.reason.clone()),
                        lifecycle_updated_unix_ms: lifecycle.map(|row| row.updated_unix_ms),
                        protocol: feature.map(|row| row.protocol.clone()),
                        category: feature.map(|row| row.category.clone()),
                        mev_score: feature.map(|row| row.mev_score),
                        urgency_score: feature.map(|row| row.urgency_score),
                        feature_engine_version: feature
                            .map(|row| row.feature_engine_version.clone()),
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
            let seen = storage
                .tx_seen()
                .iter()
                .rev()
                .find(|seen| seen.hash == hash)?;
            let full = storage.tx_full().iter().rev().find(|row| row.hash == hash);
            let feature = storage
                .tx_features()
                .iter()
                .rev()
                .find(|row| row.hash == hash);
            let lifecycle = storage
                .tx_lifecycle()
                .iter()
                .rev()
                .find(|row| row.hash == hash);
            let fallback_lifecycle = lifecycle
                .is_none()
                .then(|| current_lifecycle(&storage.list_events(), hash));
            let (lifecycle_status, lifecycle_reason) = lifecycle
                .map(|row| (Some(row.status.clone()), row.reason.clone()))
                .unwrap_or_else(|| {
                    fallback_lifecycle
                        .flatten()
                        .as_ref()
                        .map(map_lifecycle_status)
                        .map(|(status, reason)| (Some(status), reason))
                        .unwrap_or((None, None))
                });
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
                lifecycle_status,
                lifecycle_reason,
                lifecycle_updated_unix_ms: lifecycle.map(|row| row.updated_unix_ms),
                protocol: feature.map(|row| row.protocol.clone()),
                category: feature.map(|row| row.category.clone()),
                mev_score: feature.map(|row| row.mev_score),
                urgency_score: feature.map(|row| row.urgency_score),
                feature_engine_version: feature.map(|row| row.feature_engine_version.clone()),
            })
        })
    }

    fn metric_snapshot(&self) -> MetricSnapshot {
        let (tx_seen_len, tx_full_len) = self
            .storage
            .read()
            .ok()
            .map(|storage| {
                (
                    storage.tx_seen().len() as u64,
                    storage.tx_full().len() as u64,
                )
            })
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
        .route("/events", get(events))
        .route("/replay", get(replay))
        .route("/propagation", get(propagation))
        .route("/metrics/snapshot", get(metrics_snapshot))
        .route("/alerts/evaluate", get(alerts_evaluate))
        .route("/features", get(features))
        .route("/features/recent", get(features_recent))
        .route("/opps", get(opportunities))
        .route("/opps/recent", get(opportunities_recent))
        .route("/dashboard/snapshot", get(dashboard_snapshot))
        .route("/tx/{hash}", get(transaction_by_hash))
        .route("/sim/{id}", get(sim_by_id))
        .route("/bundle/{id}", get(bundle_by_id))
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
        .route("/metrics", get(metrics_prometheus))
        .merge(protected)
        .layer(cors)
        .with_state(state)
}

pub fn default_state_with_runtime() -> (AppState, RuntimeBootstrap) {
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
    let ingest_mode = resolve_ingest_source_mode(env::var("VIZ_API_INGEST_MODE").ok().as_deref());

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

    let api_auth = ApiAuthConfig::from_env();
    if api_auth.enabled && api_auth.api_keys.is_empty() {
        tracing::warn!(
            "api auth enabled but no API keys configured; all protected routes will return unauthorized"
        );
    }
    let api_rate_limiter = ApiRateLimiter::new(api_auth.requests_per_minute);
    let live_rpc_chain_status_provider = Arc::new(live_rpc_chain_status_snapshot)
        as Arc<dyn Fn() -> Vec<LiveRpcChainStatus> + Send + Sync>;
    let live_rpc_drop_metrics_provider = Arc::new(live_rpc_drop_metrics_snapshot)
        as Arc<dyn Fn() -> LiveRpcDropMetricsSnapshot + Send + Sync>;

    let state = AppState {
        provider: Arc::new(InMemoryVizProvider::new(
            storage.clone(),
            Arc::new(propagation),
            1,
        )),
        downsample_limit: 1_000,
        relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
        alert_thresholds: AlertThresholdConfig::default(),
        api_auth,
        api_rate_limiter,
        live_rpc_chain_status_provider,
        live_rpc_drop_metrics_provider,
    };

    (
        state,
        RuntimeBootstrap {
            storage: storage.clone(),
            writer,
            live_rpc_config,
            ingest_mode,
        },
    )
}

pub fn default_state() -> AppState {
    default_state_with_runtime().0
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

fn map_lifecycle_status(status: &TxLifecycleStatus) -> (String, Option<String>) {
    match status {
        TxLifecycleStatus::Pending => ("pending".to_owned(), None),
        TxLifecycleStatus::Replaced { by } => ("replaced".to_owned(), Some(format_bytes(by))),
        TxLifecycleStatus::Dropped { reason } => ("dropped".to_owned(), Some(reason.clone())),
        TxLifecycleStatus::ConfirmedProvisional {
            block_number,
            block_hash,
        } => (
            "confirmed_provisional".to_owned(),
            Some(format!("block={block_number}:{}", format_bytes(block_hash))),
        ),
        TxLifecycleStatus::ConfirmedFinal {
            block_number,
            block_hash,
        } => (
            "confirmed_final".to_owned(),
            Some(format!("block={block_number}:{}", format_bytes(block_hash))),
        ),
    }
}

fn event_payload_type(payload: &EventPayload) -> &str {
    match payload {
        EventPayload::TxSeen(_) => "TxSeen",
        EventPayload::TxFetched(_) => "TxFetched",
        EventPayload::TxDecoded(_) => "TxDecoded",
        EventPayload::OppDetected(_) => "OppDetected",
        EventPayload::SimCompleted(_) => "SimCompleted",
        EventPayload::BundleSubmitted(_) => "BundleSubmitted",
        EventPayload::TxReplaced(_) => "TxReplaced",
        EventPayload::TxDropped(_) => "TxDropped",
        EventPayload::TxConfirmedProvisional(_) => "TxConfirmedProvisional",
        EventPayload::TxConfirmedFinal(_) => "TxConfirmedFinal",
        EventPayload::TxReorged(_) => "TxReorged",
    }
}

fn parse_event_type_filters(raw: Option<&str>) -> Vec<String> {
    let mut filters = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    filters.sort_unstable();
    filters.dedup();
    filters
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
    if !trimmed.len().is_multiple_of(2) {
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

async fn metrics_prometheus(State(state): State<AppState>) -> impl IntoResponse {
    let body = render_prometheus_metrics(&state);
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

async fn alerts_evaluate(State(state): State<AppState>) -> Json<AlertDecisions> {
    let snapshot = state.provider.metric_snapshot();
    Json(evaluate_alerts(&snapshot, &state.alert_thresholds))
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ReplayQuery {
    from: Option<u64>,
    to: Option<u64>,
    stride: Option<usize>,
}

async fn replay(
    State(state): State<AppState>,
    Query(query): Query<ReplayQuery>,
) -> impl IntoResponse {
    let (from_seq_id, to_seq_id) = match (query.from, query.to) {
        (Some(from_seq_id), Some(to_seq_id)) => {
            if from_seq_id <= to_seq_id {
                (from_seq_id, to_seq_id)
            } else {
                (to_seq_id, from_seq_id)
            }
        }
        (None, None) => {
            let values = state.provider.replay_points();
            return Json(downsample(&values, state.downsample_limit)).into_response();
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    let events = collect_events_up_to_seq(state.provider.as_ref(), to_seq_id);
    let summary = match replay_diff_summary(&events, from_seq_id, to_seq_id) {
        Some(summary) => summary,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let checkpoint = match replay::lifecycle_snapshot(&events, from_seq_id) {
        Some(checkpoint) => checkpoint,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let stride = query.stride.unwrap_or(1).clamp(1, 5_000);
    let frames = replay_from_checkpoint(&events, &checkpoint, stride)
        .into_iter()
        .filter(|frame| frame.seq_hi <= to_seq_id)
        .map(|frame| ReplayPoint {
            seq_hi: frame.seq_hi,
            timestamp_unix_ms: frame.timestamp_unix_ms,
            pending_count: frame.pending.len() as u32,
        })
        .collect::<Vec<_>>();

    let response = ReplayRangeResponse {
        from_seq_id: summary.from_seq_id,
        to_seq_id: summary.to_seq_id,
        from_checkpoint_hash: format_bytes(&summary.from_checkpoint_hash),
        to_checkpoint_hash: format_bytes(&summary.to_checkpoint_hash),
        summary: ReplayRangeDiffSummary {
            from_pending_count: summary.from_pending_count,
            to_pending_count: summary.to_pending_count,
            added_pending_count: summary.added_pending.len(),
            removed_pending_count: summary.removed_pending.len(),
            added_pending: summary
                .added_pending
                .iter()
                .map(|hash| format_bytes(hash))
                .collect(),
            removed_pending: summary
                .removed_pending
                .iter()
                .map(|hash| format_bytes(hash))
                .collect(),
        },
        frames,
    };
    Json(response).into_response()
}

#[derive(Clone, Debug, Default, Deserialize)]
struct EventsQuery {
    after: Option<u64>,
    types: Option<String>,
    limit: Option<usize>,
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Json<Vec<EventEnvelope>> {
    let after_seq_id = query.after.unwrap_or(0);
    let limit = query.limit.unwrap_or(1_000).clamp(1, 5_000);
    let event_types = parse_event_type_filters(query.types.as_deref());
    Json(state.provider.events(after_seq_id, &event_types, limit))
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
    let chain_id = query.chain_id;
    let scan_limit = chain_filter_scan_limit(limit, chain_id);
    Json(filter_feature_details_by_chain(
        state.provider.feature_details(scan_limit),
        chain_id,
        limit,
    ))
}

#[derive(Clone, Debug, Default, Deserialize)]
struct TransactionsQuery {
    limit: Option<usize>,
    min_score: Option<u32>,
    status: Option<String>,
    chain_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct DashboardSnapshotQuery {
    tx_limit: Option<usize>,
    feature_limit: Option<usize>,
    opp_limit: Option<usize>,
    min_score: Option<u32>,
    replay_limit: Option<usize>,
    chain_id: Option<u64>,
}

async fn dashboard_snapshot(
    State(state): State<AppState>,
    Query(query): Query<DashboardSnapshotQuery>,
) -> Json<DashboardSnapshot> {
    let tx_limit = query.tx_limit.unwrap_or(250).clamp(1, 500);
    let feature_limit = query.feature_limit.unwrap_or(600).clamp(1, 2_000);
    let opp_limit = query.opp_limit.unwrap_or(600).clamp(1, 2_000);
    let min_score = query.min_score.unwrap_or(0);
    let chain_id = query.chain_id;
    let replay_limit = query
        .replay_limit
        .unwrap_or(state.downsample_limit)
        .clamp(50, 5_000);

    let replay_points = state.provider.replay_points();
    let latest_seq_id = replay_points.last().map(|point| point.seq_hi).unwrap_or(0);
    let feature_scan_limit = chain_filter_scan_limit(feature_limit, chain_id);
    let opp_scan_limit = chain_filter_scan_limit(opp_limit, chain_id);
    let tx_scan_limit = chain_filter_scan_limit(tx_limit, chain_id);

    Json(DashboardSnapshot {
        replay: downsample(&replay_points, replay_limit),
        propagation: downsample(&state.provider.propagation_edges(), state.downsample_limit),
        opportunities: filter_opportunities_by_chain(
            state.provider.opportunities(opp_scan_limit, min_score),
            chain_id,
            opp_limit,
        ),
        feature_summary: downsample(&state.provider.feature_summary(), state.downsample_limit),
        feature_details: filter_feature_details_by_chain(
            state.provider.feature_details(feature_scan_limit),
            chain_id,
            feature_limit,
        ),
        transactions: filter_transaction_summaries_by_chain(
            state.provider.as_ref(),
            state.provider.recent_transactions(tx_scan_limit),
            chain_id,
            tx_limit,
        ),
        chain_ingest_status: (state.live_rpc_chain_status_provider)(),
        market_stats: state.provider.market_stats(),
        latest_seq_id,
    })
}

async fn transactions(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<TransactionSummary>> {
    let limit = query.limit.unwrap_or(25).clamp(1, 200);
    let chain_id = query.chain_id;
    let scan_limit = chain_filter_scan_limit(limit, chain_id);
    Json(filter_transaction_summaries_by_chain(
        state.provider.as_ref(),
        state.provider.recent_transactions(scan_limit),
        chain_id,
        limit,
    ))
}

async fn opportunities_recent(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<OpportunityDetail>> {
    Json(filtered_opportunities(&state, &query))
}

async fn opportunities(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<OpportunityDetail>> {
    Json(filtered_opportunities(&state, &query))
}

fn filtered_opportunities(state: &AppState, query: &TransactionsQuery) -> Vec<OpportunityDetail> {
    let limit = query.limit.unwrap_or(100).clamp(1, 5_000);
    let min_score = query.min_score.unwrap_or(0);
    let chain_id = query.chain_id;
    let scan_limit = chain_filter_scan_limit(limit, chain_id);
    let values = state.provider.opportunities(scan_limit, min_score);
    let status_filter = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let filtered = match status_filter {
        Some(status) => values
            .into_iter()
            .filter(|row| row.status.eq_ignore_ascii_case(status))
            .collect::<Vec<_>>(),
        None => values,
    };
    filter_opportunities_by_chain(filtered, chain_id, limit)
}

fn chain_filter_scan_limit(limit: usize, chain_id: Option<u64>) -> usize {
    if chain_id.is_some() {
        limit.saturating_mul(4).clamp(limit, 20_000)
    } else {
        limit
    }
}

fn chain_matches_filter(record_chain_id: Option<u64>, chain_id_filter: Option<u64>) -> bool {
    match chain_id_filter {
        Some(chain_id) => record_chain_id == Some(chain_id),
        None => true,
    }
}

fn filter_feature_details_by_chain(
    values: Vec<FeatureDetail>,
    chain_id: Option<u64>,
    limit: usize,
) -> Vec<FeatureDetail> {
    values
        .into_iter()
        .filter(|row| chain_matches_filter(row.chain_id, chain_id))
        .take(limit)
        .collect()
}

fn filter_opportunities_by_chain(
    values: Vec<OpportunityDetail>,
    chain_id: Option<u64>,
    limit: usize,
) -> Vec<OpportunityDetail> {
    values
        .into_iter()
        .filter(|row| chain_matches_filter(row.chain_id, chain_id))
        .take(limit)
        .collect()
}

fn filter_transaction_details_by_chain(
    values: Vec<TransactionDetail>,
    chain_id: Option<u64>,
    limit: usize,
) -> Vec<TransactionDetail> {
    values
        .into_iter()
        .filter(|row| chain_matches_filter(row.chain_id, chain_id))
        .take(limit)
        .collect()
}

fn filter_transaction_summaries_by_chain(
    provider: &dyn VizDataProvider,
    values: Vec<TransactionSummary>,
    chain_id: Option<u64>,
    limit: usize,
) -> Vec<TransactionSummary> {
    values
        .into_iter()
        .filter(|row| {
            if chain_id.is_none() {
                return true;
            }
            provider
                .transaction_detail_by_hash(&row.hash)
                .map(|detail| chain_matches_filter(detail.chain_id, chain_id))
                .unwrap_or(false)
        })
        .take(limit)
        .collect()
}

async fn transactions_all(
    State(state): State<AppState>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<TransactionDetail>> {
    let limit = query.limit.unwrap_or(1_000).clamp(1, 5_000);
    let chain_id = query.chain_id;
    let scan_limit = chain_filter_scan_limit(limit, chain_id);
    Json(filter_transaction_details_by_chain(
        state.provider.transaction_details(scan_limit),
        chain_id,
        limit,
    ))
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

async fn bundle_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BundleDetail>, StatusCode> {
    let relay = state
        .relay_dry_run_status
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let latest = relay.latest.ok_or(StatusCode::NOT_FOUND)?;
    let bundle_id = bundle_id_for_result(&latest);
    if id != "latest" && id != bundle_id {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(BundleDetail {
        id: bundle_id,
        relay_url: latest.relay_url,
        accepted: latest.accepted,
        final_state: latest.final_state,
        attempt_count: latest.attempts.len(),
        started_unix_ms: latest.started_unix_ms,
        finished_unix_ms: latest.finished_unix_ms,
    }))
}

async fn sim_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SimDetail>, StatusCode> {
    let relay = state
        .relay_dry_run_status
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let latest = relay.latest.ok_or(StatusCode::NOT_FOUND)?;

    let bundle_id = bundle_id_for_result(&latest);
    let sim_id = sim_id_for_result(&latest);
    if id != "latest" && id != sim_id {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(SimDetail {
        id: sim_id,
        bundle_id,
        status: if latest.accepted {
            "ok".to_owned()
        } else {
            "fail".to_owned()
        },
        relay_url: latest.relay_url,
        attempt_count: latest.attempts.len(),
        accepted: latest.accepted,
        fail_category: if latest.accepted {
            None
        } else {
            Some("relay_exhausted".to_owned())
        },
        started_unix_ms: latest.started_unix_ms,
        finished_unix_ms: latest.finished_unix_ms,
    }))
}

fn render_prometheus_metrics(state: &AppState) -> String {
    let snapshot = state.provider.metric_snapshot();
    let relay = state
        .relay_dry_run_status
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let relay_success_rate = if relay.total_submissions == 0 {
        0.0
    } else {
        relay.total_accepted as f64 / relay.total_submissions as f64
    };

    let mut out = String::new();
    out.push_str("# TYPE mempulse_ingest_queue_depth gauge\n");
    out.push_str(&format!(
        "mempulse_ingest_queue_depth {}\n",
        snapshot.queue_depth_current
    ));
    out.push_str("# TYPE mempulse_ingest_queue_capacity gauge\n");
    out.push_str(&format!(
        "mempulse_ingest_queue_capacity {}\n",
        snapshot.queue_depth_capacity
    ));
    out.push_str("# TYPE mempulse_ingest_decode_fail_total counter\n");
    out.push_str(&format!(
        "mempulse_ingest_decode_fail_total {}\n",
        snapshot.tx_decode_fail_total
    ));
    out.push_str("# TYPE mempulse_ingest_decode_total counter\n");
    out.push_str(&format!(
        "mempulse_ingest_decode_total {}\n",
        snapshot.tx_decode_total
    ));
    out.push_str("# TYPE mempulse_ingest_lag_ms gauge\n");
    out.push_str(&format!(
        "mempulse_ingest_lag_ms {}\n",
        snapshot.ingest_lag_ms
    ));
    out.push_str("# TYPE mempulse_ingest_tx_per_sec_current gauge\n");
    out.push_str(&format!(
        "mempulse_ingest_tx_per_sec_current {}\n",
        snapshot.tx_per_sec_current
    ));
    out.push_str("# TYPE mempulse_ingest_tx_per_sec_baseline gauge\n");
    out.push_str(&format!(
        "mempulse_ingest_tx_per_sec_baseline {}\n",
        snapshot.tx_per_sec_baseline
    ));
    out.push_str("# TYPE mempulse_ingest_drops_total counter\n");
    out.push_str(&format!(
        "mempulse_ingest_drops_total{{reason=\"decode_fail\"}} {}\n",
        snapshot.tx_decode_fail_total
    ));
    let drop_metrics = (state.live_rpc_drop_metrics_provider)();
    out.push_str(&format!(
        "mempulse_ingest_drops_total{{reason=\"storage_queue_full\"}} {}\n",
        drop_metrics.storage_queue_full
    ));
    out.push_str(&format!(
        "mempulse_ingest_drops_total{{reason=\"storage_queue_closed\"}} {}\n",
        drop_metrics.storage_queue_closed
    ));
    out.push_str(&format!(
        "mempulse_ingest_drops_total{{reason=\"invalid_pending_hash\"}} {}\n",
        drop_metrics.invalid_pending_hash
    ));

    // Stub values until replay runtime exports these counters directly.
    out.push_str("# TYPE mempulse_replay_lag_events gauge\n");
    out.push_str("mempulse_replay_lag_events 0\n");
    out.push_str("# TYPE mempulse_replay_checkpoint_duration_ms gauge\n");
    out.push_str("mempulse_replay_checkpoint_duration_ms 0\n");
    out.push_str("# TYPE mempulse_replay_reorg_depth gauge\n");
    out.push_str("mempulse_replay_reorg_depth 0\n");

    // Stub values until simulator exports categorized metrics directly.
    out.push_str("# TYPE mempulse_sim_latency_ms gauge\n");
    out.push_str("mempulse_sim_latency_ms 0\n");
    out.push_str("# TYPE mempulse_sim_fail_total counter\n");
    out.push_str("mempulse_sim_fail_total{category=\"unknown\"} 0\n");

    out.push_str("# TYPE mempulse_relay_success_rate gauge\n");
    out.push_str(&format!(
        "mempulse_relay_success_rate {relay_success_rate:.6}\n"
    ));
    out.push_str("# TYPE mempulse_relay_bundle_included_total counter\n");
    out.push_str(&format!(
        "mempulse_relay_bundle_included_total {}\n",
        relay.total_accepted
    ));
    out.push_str("# TYPE mempulse_relay_bundle_filtered_total counter\n");
    out.push_str(&format!(
        "mempulse_relay_bundle_filtered_total{{reason=\"dry_run_failed\"}} {}\n",
        relay.total_failed
    ));
    out
}

fn collect_events_up_to_seq(provider: &dyn VizDataProvider, to_seq_id: u64) -> Vec<EventEnvelope> {
    const PAGE_SIZE: usize = 2_000;
    const MAX_SCAN_EVENTS: usize = 200_000;

    let mut after_seq_id = 0_u64;
    let mut out = Vec::new();
    while out.len() < MAX_SCAN_EVENTS {
        let page = provider.events(after_seq_id, &[], PAGE_SIZE);
        if page.is_empty() {
            break;
        }

        let mut progressed = false;
        for event in page {
            let seq_id = event.seq_id;
            if event.seq_id <= after_seq_id {
                continue;
            }
            progressed = true;
            after_seq_id = seq_id;
            if seq_id <= to_seq_id {
                out.push(event);
            }
            if seq_id >= to_seq_id {
                return out;
            }
        }

        if !progressed {
            break;
        }
    }

    out
}

fn bundle_id_for_result(result: &RelayDryRunResult) -> String {
    format!(
        "bundle-{}-{}-{}",
        result.started_unix_ms,
        result.finished_unix_ms,
        result.attempts.len()
    )
}

fn sim_id_for_result(result: &RelayDryRunResult) -> String {
    format!(
        "sim-{}-{}-{}",
        result.started_unix_ms,
        result.finished_unix_ms,
        result.attempts.len()
    )
}

#[derive(Clone, Debug, Default, Deserialize)]
struct StreamQuery {
    after: Option<u64>,
    limit: Option<usize>,
    interval_ms: Option<u64>,
}

async fn stream(
    State(state): State<AppState>,
    Query(query): Query<StreamQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let hello = StreamHello {
        event: "hello".to_owned(),
        message: "viz-api stream connected".to_owned(),
        replay_points: state.provider.replay_points().len(),
        propagation_edges: state.provider.propagation_edges().len(),
    };
    let after_seq_id = query.after.unwrap_or(0);
    let batch_limit = query.limit.unwrap_or(256).clamp(1, 5_000);
    let interval_ms = query.interval_ms.unwrap_or(250).clamp(50, 5_000);
    let provider = state.provider.clone();
    ws.on_upgrade(move |socket| {
        handle_socket(
            socket,
            hello,
            provider,
            after_seq_id,
            batch_limit,
            interval_ms,
        )
    })
}

async fn handle_socket(
    mut socket: WebSocket,
    hello: StreamHello,
    provider: Arc<dyn VizDataProvider>,
    mut after_seq_id: u64,
    batch_limit: usize,
    interval_ms: u64,
) {
    if let Ok(payload) = serde_json::to_string(&hello)
        && socket.send(Message::Text(payload.into())).await.is_err()
    {
        return;
    }

    let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let events = provider.events(after_seq_id, &[], batch_limit);
                if events.is_empty() {
                    continue;
                }
                for event in events {
                    if event.seq_id <= after_seq_id {
                        continue;
                    }
                    after_seq_id = event.seq_id;
                    let payload = match serde_json::to_string(&event) {
                        Ok(payload) => payload,
                        Err(_) => continue,
                    };
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
            }
            maybe_message = socket.recv() => match maybe_message {
                None => break,
                Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Ping(data))) => {
                    if socket.send(Message::Pong(data)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
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
        fn events(
            &self,
            after_seq_id: u64,
            event_types: &[String],
            limit: usize,
        ) -> Vec<EventEnvelope> {
            let values = vec![
                EventEnvelope {
                    seq_id: 1,
                    ingest_ts_unix_ms: 1_700_000_000_000,
                    ingest_ts_mono_ns: 1_700_000_000_000_000_000,
                    source_id: common::SourceId::new("mock"),
                    payload: EventPayload::TxSeen(TxSeen {
                        hash: [0x01; 32],
                        peer_id: "rpc-ws".to_owned(),
                        seen_at_unix_ms: 1_700_000_000_000,
                        seen_at_mono_ns: 1_700_000_000_000_000_000,
                    }),
                },
                EventEnvelope {
                    seq_id: 2,
                    ingest_ts_unix_ms: 1_700_000_000_050,
                    ingest_ts_mono_ns: 1_700_000_000_050_000_000,
                    source_id: common::SourceId::new("mock"),
                    payload: EventPayload::TxFetched(TxFetched {
                        hash: [0x02; 32],
                        fetched_at_unix_ms: 1_700_000_000_050,
                    }),
                },
                EventEnvelope {
                    seq_id: 3,
                    ingest_ts_unix_ms: 1_700_000_000_100,
                    ingest_ts_mono_ns: 1_700_000_000_100_000_000,
                    source_id: common::SourceId::new("mock"),
                    payload: EventPayload::TxDecoded(TxDecoded {
                        hash: [0x03; 32],
                        tx_type: 2,
                        sender: [0xaa; 20],
                        nonce: 1,
                        chain_id: Some(1),
                        to: None,
                        value_wei: None,
                        gas_limit: None,
                        gas_price_wei: None,
                        max_fee_per_gas_wei: None,
                        max_priority_fee_per_gas_wei: None,
                        max_fee_per_blob_gas_wei: None,
                        calldata_len: Some(4),
                    }),
                },
            ];
            values
                .into_iter()
                .filter(|event| event.seq_id > after_seq_id)
                .filter(|event| {
                    event_types.is_empty()
                        || event_types.iter().any(|kind| {
                            event_payload_type(&event.payload).eq_ignore_ascii_case(kind)
                        })
                })
                .take(limit)
                .collect()
        }

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
                    chain_id: Some(1),
                    mev_score: 72,
                    urgency_score: 18,
                    method_selector: Some("0x38ed1739".to_owned()),
                    feature_engine_version: "feature-engine.v1".to_owned(),
                },
                FeatureDetail {
                    hash: "0x02".to_owned(),
                    protocol: "erc20".to_owned(),
                    category: "transfer".to_owned(),
                    chain_id: Some(1),
                    mev_score: 18,
                    urgency_score: 7,
                    method_selector: Some("0xa9059cbb".to_owned()),
                    feature_engine_version: "feature-engine.v1".to_owned(),
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

        fn opportunities(&self, limit: usize, min_score: u32) -> Vec<OpportunityDetail> {
            let values = vec![
                OpportunityDetail {
                    tx_hash: "0x11".to_owned(),
                    status: "detected".to_owned(),
                    strategy: "SandwichCandidate".to_owned(),
                    score: 12_000,
                    protocol: "uniswap-v2".to_owned(),
                    category: "swap".to_owned(),
                    chain_id: Some(1),
                    feature_engine_version: "feature-engine.v1".to_owned(),
                    scorer_version: "scorer.v1".to_owned(),
                    strategy_version: "strategy.sandwich.v1".to_owned(),
                    reasons: vec!["mev_score=90*120".to_owned()],
                    detected_unix_ms: 1_700_000_000_111,
                },
                OpportunityDetail {
                    tx_hash: "0x22".to_owned(),
                    status: "detected".to_owned(),
                    strategy: "BackrunCandidate".to_owned(),
                    score: 9_500,
                    protocol: "uniswap-v3".to_owned(),
                    category: "swap".to_owned(),
                    chain_id: Some(1),
                    feature_engine_version: "feature-engine.v1".to_owned(),
                    scorer_version: "scorer.v1".to_owned(),
                    strategy_version: "strategy.backrun.v1".to_owned(),
                    reasons: vec!["urgency_score=20*20".to_owned()],
                    detected_unix_ms: 1_700_000_000_101,
                },
            ];
            values
                .into_iter()
                .filter(|row| row.score >= min_score)
                .take(limit)
                .collect()
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
                    lifecycle_status: Some("pending".to_owned()),
                    lifecycle_reason: Some("reorg-reopened".to_owned()),
                    lifecycle_updated_unix_ms: Some(1_700_000_000_111),
                    protocol: Some("uniswap-v2".to_owned()),
                    category: Some("swap".to_owned()),
                    mev_score: Some(72),
                    urgency_score: Some(18),
                    feature_engine_version: Some("feature-engine.v1".to_owned()),
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
                    lifecycle_status: None,
                    lifecycle_reason: None,
                    lifecycle_updated_unix_ms: None,
                    protocol: None,
                    category: None,
                    mev_score: None,
                    urgency_score: None,
                    feature_engine_version: None,
                },
            ];
            values.into_iter().take(limit).collect()
        }

        fn transaction_detail_by_hash(&self, hash: &str) -> Option<TransactionDetail> {
            self.transaction_details(100)
                .into_iter()
                .find(|row| row.hash == hash)
        }

        fn market_stats(&self) -> MarketStats {
            MarketStats {
                total_signal_volume: 9_536,
                total_tx_count: 9_536,
                low_risk_count: 9_201,
                medium_risk_count: 286,
                high_risk_count: 49,
                success_rate_bps: 9_949,
            }
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
        let live_rpc_chain_status_provider = Arc::new(Vec::<LiveRpcChainStatus>::new)
            as Arc<dyn Fn() -> Vec<LiveRpcChainStatus> + Send + Sync>;
        let live_rpc_drop_metrics_provider = Arc::new(LiveRpcDropMetricsSnapshot::default)
            as Arc<dyn Fn() -> LiveRpcDropMetricsSnapshot + Send + Sync>;
        AppState {
            provider: Arc::new(MockProvider),
            downsample_limit: limit,
            relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
            alert_thresholds: AlertThresholdConfig::default(),
            api_rate_limiter: ApiRateLimiter::new(api_auth.requests_per_minute),
            api_auth,
            live_rpc_chain_status_provider,
            live_rpc_drop_metrics_provider,
        }
    }

    fn test_state_with_relay(limit: usize, relay_status: RelayDryRunStatus) -> AppState {
        let api_auth = ApiAuthConfig::default();
        let live_rpc_chain_status_provider = Arc::new(Vec::<LiveRpcChainStatus>::new)
            as Arc<dyn Fn() -> Vec<LiveRpcChainStatus> + Send + Sync>;
        let live_rpc_drop_metrics_provider = Arc::new(LiveRpcDropMetricsSnapshot::default)
            as Arc<dyn Fn() -> LiveRpcDropMetricsSnapshot + Send + Sync>;
        AppState {
            provider: Arc::new(MockProvider),
            downsample_limit: limit,
            relay_dry_run_status: Arc::new(RwLock::new(relay_status)),
            alert_thresholds: AlertThresholdConfig::default(),
            api_rate_limiter: ApiRateLimiter::new(api_auth.requests_per_minute),
            api_auth,
            live_rpc_chain_status_provider,
            live_rpc_drop_metrics_provider,
        }
    }

    fn seeded_relay_status() -> RelayDryRunStatus {
        let result = RelayDryRunResult {
            relay_url: "https://relay.example".to_owned(),
            accepted: false,
            final_state: "exhausted".to_owned(),
            attempts: vec![RelayAttemptTrace {
                attempt: 1,
                endpoint: "https://relay.example".to_owned(),
                http_status: Some(500),
                error: Some("timeout".to_owned()),
                latency_ms: 300,
                backoff_ms: 50,
            }],
            started_unix_ms: 1_700_000_000_010,
            finished_unix_ms: 1_700_000_000_020,
        };
        let mut status = RelayDryRunStatus::default();
        status.record(result);
        status
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
    async fn metrics_prometheus_route_returns_prometheus_text_series() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(content_type.starts_with("text/plain; version=0.0.4"));

        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload = String::from_utf8(body.to_vec()).unwrap();
        assert!(payload.contains("mempulse_ingest_queue_depth"));
        assert!(payload.contains("mempulse_ingest_drops_total{reason=\"decode_fail\"}"));
        assert!(payload.contains("mempulse_ingest_lag_ms"));
        assert!(payload.contains("mempulse_ingest_decode_total"));
        assert!(payload.contains("mempulse_ingest_tx_per_sec_current"));
        assert!(payload.contains("mempulse_replay_lag_events"));
        assert!(payload.contains("mempulse_sim_fail_total{category=\"unknown\"}"));
        assert!(payload.contains("mempulse_relay_success_rate"));
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
    async fn replay_route_from_to_returns_checkpoint_hash_and_diff_summary() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/replay?from=1&to=3&stride=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["from_seq_id"], serde_json::json!(1));
        assert_eq!(payload["to_seq_id"], serde_json::json!(3));
        assert_eq!(
            payload["summary"]["added_pending_count"],
            serde_json::json!(1)
        );
        assert_eq!(
            payload["summary"]["removed_pending_count"],
            serde_json::json!(0)
        );
        assert_eq!(
            payload["frames"].as_array().map(|values| values.len()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn events_route_filters_by_after_and_types() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events?after=1&types=TxDecoded&limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<EventEnvelope> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].seq_id, 3);
        assert!(matches!(payload[0].payload, EventPayload::TxDecoded(_)));
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
        assert_eq!(
            resolve_ingest_source_mode(Some("p2p")),
            IngestSourceMode::P2p
        );
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
    async fn dashboard_snapshot_route_returns_aggregated_payload() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri(
                        "/dashboard/snapshot?tx_limit=1&feature_limit=1&opp_limit=1&replay_limit=2",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: DashboardSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.transactions.len(), 1);
        assert_eq!(payload.feature_details.len(), 1);
        assert_eq!(payload.opportunities.len(), 1);
        assert!(!payload.replay.is_empty());
        assert_eq!(payload.latest_seq_id, 3);
        assert_eq!(payload.market_stats.total_signal_volume, 9_536);
        assert_eq!(payload.market_stats.success_rate_bps, 9_949);
        let _ = payload.chain_ingest_status;
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
    async fn tx_route_alias_returns_detail_row() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/tx/0x01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: TransactionDetail = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.hash, "0x01");
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
        assert_eq!(payload[0].feature_engine_version, "feature-engine.v1");
    }

    #[tokio::test]
    async fn opportunities_recent_route_applies_min_score_filter() {
        let app = build_router(test_state(100));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/opps/recent?limit=10&min_score=10000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<OpportunityDetail> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].strategy, "SandwichCandidate");
        assert_eq!(payload[0].scorer_version, "scorer.v1");
    }

    #[tokio::test]
    async fn opps_route_applies_status_filter() {
        let app = build_router(test_state(100));

        let detected = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/opps?limit=10&status=detected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detected.status(), StatusCode::OK);
        let body = to_bytes(detected.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<OpportunityDetail> = serde_json::from_slice(&body).unwrap();
        assert!(!payload.is_empty());

        let invalidated = app
            .oneshot(
                Request::builder()
                    .uri("/opps?limit=10&status=invalidated")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalidated.status(), StatusCode::OK);
        let body = to_bytes(invalidated.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let payload: Vec<OpportunityDetail> = serde_json::from_slice(&body).unwrap();
        assert!(payload.is_empty());
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

    #[tokio::test]
    async fn bundle_route_returns_latest_bundle_detail() {
        let app = build_router(test_state_with_relay(100, seeded_relay_status()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/bundle/latest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: BundleDetail = serde_json::from_slice(&body).unwrap();
        assert!(!payload.id.is_empty());
        assert_eq!(payload.relay_url, "https://relay.example");
        assert_eq!(payload.attempt_count, 1);
        assert!(!payload.accepted);
    }

    #[tokio::test]
    async fn sim_route_returns_latest_sim_detail() {
        let app = build_router(test_state_with_relay(100, seeded_relay_status()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/sim/latest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: SimDetail = serde_json::from_slice(&body).unwrap();
        assert!(!payload.id.is_empty());
        assert_eq!(payload.relay_url, "https://relay.example");
        assert_eq!(payload.status, "fail");
        assert_eq!(payload.fail_category.as_deref(), Some("relay_exhausted"));
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
    fn in_memory_provider_feature_summary_is_aggregated_and_sorted() {
        let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
        {
            let mut guard = storage.write().expect("write lock");
            for (seq, protocol, category) in [
                (11_u64, "uniswap-v3", "swap"),
                (22_u64, "aave-v3", "borrow"),
                (33_u64, "aave-v3", "borrow"),
                (44_u64, "uniswap-v3", "swap"),
                (55_u64, "aave-v3", "borrow"),
            ] {
                guard.upsert_tx_features(storage::TxFeaturesRecord {
                    hash: hash_from_seq(seq),
                    chain_id: Some(1),
                    protocol: protocol.to_owned(),
                    category: category.to_owned(),
                    mev_score: 50,
                    urgency_score: 10,
                    method_selector: None,
                    feature_engine_version: "feature-engine.v1".to_owned(),
                });
            }
        }

        let provider = InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1);
        let summary = provider.feature_summary();

        assert_eq!(
            summary,
            vec![
                FeatureSummary {
                    protocol: "aave-v3".to_owned(),
                    category: "borrow".to_owned(),
                    count: 3,
                },
                FeatureSummary {
                    protocol: "uniswap-v3".to_owned(),
                    category: "swap".to_owned(),
                    count: 2,
                },
            ]
        );
    }

    #[test]
    fn in_memory_provider_propagation_edges_are_sorted_deterministically() {
        let provider = InMemoryVizProvider::new(
            Arc::new(RwLock::new(InMemoryStorage::default())),
            Arc::new(vec![
                PropagationEdge {
                    source: "peer-b".to_owned(),
                    destination: "peer-c".to_owned(),
                    p50_delay_ms: 10,
                    p99_delay_ms: 20,
                },
                PropagationEdge {
                    source: "peer-a".to_owned(),
                    destination: "peer-z".to_owned(),
                    p50_delay_ms: 5,
                    p99_delay_ms: 15,
                },
                PropagationEdge {
                    source: "peer-a".to_owned(),
                    destination: "peer-b".to_owned(),
                    p50_delay_ms: 2,
                    p99_delay_ms: 9,
                },
            ]),
            1,
        );

        let edges = provider.propagation_edges();
        assert_eq!(edges.len(), 3);
        assert_eq!(
            edges
                .iter()
                .map(|edge| (edge.source.as_str(), edge.destination.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("peer-a", "peer-b"),
                ("peer-a", "peer-z"),
                ("peer-b", "peer-c"),
            ]
        );
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
            guard.upsert_tx_features(storage::TxFeaturesRecord {
                hash,
                chain_id: Some(1),
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                mev_score: 77,
                urgency_score: 18,
                method_selector: Some([0x38, 0xed, 0x17, 0x39]),
                feature_engine_version: "feature-engine.v1".to_owned(),
            });
            guard.upsert_tx_lifecycle(storage::TxLifecycleRecord {
                hash,
                status: "pending".to_owned(),
                reason: Some("reorg-reopened".to_owned()),
                updated_unix_ms: 1_700_000_000_123,
            });
        }

        let provider = InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1);
        let detail = provider
            .transaction_detail_by_hash(&format_bytes(&hash))
            .expect("detail row");

        assert_eq!(detail.peer, "rpc-ws");
        assert_eq!(detail.nonce, Some(42));
        assert_eq!(detail.raw_tx_len, Some(3));
        assert_eq!(detail.lifecycle_status.as_deref(), Some("pending"));
        assert_eq!(detail.lifecycle_reason.as_deref(), Some("reorg-reopened"));
        assert_eq!(detail.protocol.as_deref(), Some("uniswap-v2"));
        assert_eq!(detail.category.as_deref(), Some("swap"));
        assert_eq!(detail.mev_score, Some(77));
        assert_eq!(detail.urgency_score, Some(18));
    }

    #[test]
    fn in_memory_provider_transaction_detail_by_hash_falls_back_to_replay_lifecycle() {
        let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
        let hash = hash_from_seq(77);
        {
            let mut guard = storage.write().expect("write lock");
            guard.append_event(EventEnvelope {
                seq_id: 1,
                ingest_ts_unix_ms: 1_700_000_000_000,
                ingest_ts_mono_ns: 1,
                source_id: common::SourceId::new("seed"),
                payload: EventPayload::TxDecoded(TxDecoded {
                    hash,
                    tx_type: 2,
                    sender: [1_u8; 20],
                    nonce: 1,
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
            });
            guard.append_event(EventEnvelope {
                seq_id: 2,
                ingest_ts_unix_ms: 1_700_000_000_010,
                ingest_ts_mono_ns: 2,
                source_id: common::SourceId::new("seed"),
                payload: EventPayload::TxConfirmedProvisional(TxConfirmed {
                    hash,
                    block_number: 1_234_567,
                    block_hash: [7_u8; 32],
                }),
            });
            guard.append_event(EventEnvelope {
                seq_id: 3,
                ingest_ts_unix_ms: 1_700_000_000_020,
                ingest_ts_mono_ns: 3,
                source_id: common::SourceId::new("seed"),
                payload: EventPayload::TxReorged(TxReorged {
                    hash,
                    old_block_hash: [7_u8; 32],
                    new_block_hash: [8_u8; 32],
                }),
            });

            guard.upsert_tx_seen(storage::TxSeenRecord {
                hash,
                peer: "rpc-ws".to_owned(),
                first_seen_unix_ms: 1_700_000_000_000,
                first_seen_mono_ns: 1_700_000_000_000_000_000,
                seen_count: 1,
            });
            guard.upsert_tx_full(storage::TxFullRecord {
                hash,
                tx_type: 2,
                sender: [1_u8; 20],
                nonce: 1,
                to: None,
                chain_id: Some(1),
                value_wei: None,
                gas_limit: None,
                gas_price_wei: None,
                max_fee_per_gas_wei: None,
                max_priority_fee_per_gas_wei: None,
                max_fee_per_blob_gas_wei: None,
                calldata_len: Some(0),
                raw_tx: Vec::new(),
            });
        }

        let provider = InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1);
        let detail = provider
            .transaction_detail_by_hash(&format_bytes(&hash))
            .expect("detail row");

        assert_eq!(detail.lifecycle_status.as_deref(), Some("pending"));
    }
}
