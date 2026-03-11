use axum::{body::Body, http::Request};
use builder::{AssemblyMetrics, AssemblySnapshot, RelayDryRunStatus};
use common::AlertThresholdConfig;
use parking_lot::RwLock;
use runtime_core::{
    RuntimeCore, RuntimeCoreConfig, RuntimeCoreDeps, RuntimeCoreStartArgs, RuntimeIngestMode,
};
use scheduler::{SchedulerMetrics, SchedulerSnapshot};
use std::sync::Arc;
use storage::{
    InMemoryStorage, PeerStatsRecord, StorageTryEnqueueError, StorageWriteHandle, StorageWriteOp,
};
use tokio::sync::mpsc;
use tower::util::ServiceExt;
use viz_api::auth::{ApiAuthConfig, ApiRateLimiter};
use viz_api::live_rpc::{
    LiveRpcChainStatus, LiveRpcDropMetricsSnapshot, LiveRpcDropReason,
    LiveRpcSearcherMetricsSnapshot, LiveRpcSimulationMetricsSnapshot,
    LiveRpcSimulationStatusSnapshot, classify_storage_enqueue_drop_reason,
};
use viz_api::{AppState, InMemoryVizProvider, VizDataProvider, build_router};

fn sample_peer_stats() -> PeerStatsRecord {
    PeerStatsRecord {
        peer: "peer-a".to_owned(),
        throughput_tps: 10,
        drop_rate_bps: 120,
        rtt_ms: 7,
    }
}

#[test]
fn queue_drop_policy_bounds_queue_and_classifies_full_vs_closed() {
    let (tx, rx) = mpsc::channel(1);
    let handle = StorageWriteHandle::from_sender(tx);
    drop(rx);

    let closed_err = handle
        .try_enqueue(StorageWriteOp::UpsertPeerStats(sample_peer_stats()))
        .expect_err("closed channel should reject enqueue");
    assert_eq!(closed_err, StorageTryEnqueueError::QueueClosed);
    assert_eq!(
        classify_storage_enqueue_drop_reason(closed_err),
        LiveRpcDropReason::StorageQueueClosed
    );

    let (tx, _rx) = mpsc::channel(1);
    let handle = StorageWriteHandle::from_sender(tx);
    handle
        .try_enqueue(StorageWriteOp::UpsertPeerStats(sample_peer_stats()))
        .expect("first enqueue fits");
    let full_err = handle
        .try_enqueue(StorageWriteOp::UpsertPeerStats(sample_peer_stats()))
        .expect_err("second enqueue should hit queue bound");
    assert_eq!(full_err, StorageTryEnqueueError::QueueFull);
    assert_eq!(
        classify_storage_enqueue_drop_reason(full_err),
        LiveRpcDropReason::StorageQueueFull
    );
}

#[tokio::test]
async fn queue_drop_policy_exposes_reasoned_drop_metrics_in_prometheus() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let (scheduler, _runtime) = scheduler::scheduler_channel(scheduler::SchedulerConfig::default())
        .expect("valid scheduler config");
    let (storage_tx, _storage_rx) = mpsc::channel(8);
    let runtime_core = RuntimeCore::start(RuntimeCoreStartArgs {
        deps: RuntimeCoreDeps {
            storage: storage.clone(),
            writer: StorageWriteHandle::from_sender(storage_tx),
            scheduler,
        },
        config: RuntimeCoreConfig {
            ingest_mode: RuntimeIngestMode::Rpc,
            rebuild_scheduler_from_rpc: false,
        },
    })
    .expect("runtime core");
    runtime_core.observe_drop_reason(LiveRpcDropReason::StorageQueueFull);
    runtime_core.observe_drop_reason(LiveRpcDropReason::StorageQueueFull);
    runtime_core.observe_drop_reason(LiveRpcDropReason::StorageQueueClosed);

    let provider: Arc<dyn VizDataProvider> =
        Arc::new(InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1));
    let runtime_core_for_drop_metrics = runtime_core.clone();
    let state = AppState {
        provider,
        dashboard_stream_broadcaster: Arc::new(
            viz_api::stream_broadcast::DashboardStreamBroadcaster::new(256, 256),
        ),
        downsample_limit: 100,
        relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
        alert_thresholds: AlertThresholdConfig::default(),
        api_auth: ApiAuthConfig::default(),
        api_rate_limiter: ApiRateLimiter::new(600),
        live_rpc_chain_status_provider: Arc::new(Vec::<LiveRpcChainStatus>::new),
        live_rpc_drop_metrics_provider: Arc::new(move || {
            runtime_core_for_drop_metrics.drop_metrics()
        })
            as Arc<dyn Fn() -> LiveRpcDropMetricsSnapshot + Send + Sync>,
        live_rpc_searcher_metrics_provider: Arc::new(LiveRpcSearcherMetricsSnapshot::default),
        live_rpc_simulation_metrics_provider: Arc::new(LiveRpcSimulationMetricsSnapshot::default),
        replay_runtime_metrics_provider: Arc::new(viz_api::ReplayRuntimeMetricsSnapshot::default),
        live_rpc_simulation_status_provider: Arc::new(|_: &str| {
            Option::<LiveRpcSimulationStatusSnapshot>::None
        }),
        scheduler_snapshot_provider: Arc::new(SchedulerSnapshot::default),
        scheduler_metrics_provider: Arc::new(SchedulerMetrics::default),
        builder_snapshot_provider: Arc::new(AssemblySnapshot::default),
        builder_metrics_provider: Arc::new(AssemblyMetrics::default),
    };
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("build metrics request"),
        )
        .await
        .expect("run metrics request");

    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read metrics payload");
    let payload = String::from_utf8(body.to_vec()).expect("metrics payload is utf8");

    assert!(payload.contains("mempulse_ingest_drops_total{reason=\"storage_queue_full\"} 2"));
    assert!(payload.contains("mempulse_ingest_drops_total{reason=\"storage_queue_closed\"} 1"));
}
