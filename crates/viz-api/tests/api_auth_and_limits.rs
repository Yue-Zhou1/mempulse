use axum::body::Body;
use axum::http::Request;
use builder::RelayDryRunStatus;
use common::{AlertThresholdConfig, MetricSnapshot};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use tower::util::ServiceExt;
use viz_api::live_rpc::{LiveRpcChainStatus, LiveRpcDropMetricsSnapshot};
use viz_api::{
    AppState, MarketStats, ReplayPoint, TransactionDetail, TransactionSummary, VizDataProvider,
    build_router,
};

#[derive(Clone)]
struct EmptyProvider;

impl VizDataProvider for EmptyProvider {
    fn events(
        &self,
        _after_seq_id: u64,
        _event_types: &[String],
        _limit: usize,
    ) -> Vec<event_log::EventEnvelope> {
        Vec::new()
    }

    fn replay_points(&self) -> Vec<ReplayPoint> {
        Vec::new()
    }

    fn latest_seq_id(&self) -> Option<u64> {
        None
    }

    fn propagation_edges(&self) -> Vec<viz_api::PropagationEdge> {
        Vec::new()
    }

    fn feature_summary(&self) -> Vec<viz_api::FeatureSummary> {
        Vec::new()
    }

    fn feature_details(&self, _limit: usize) -> Vec<viz_api::FeatureDetail> {
        Vec::new()
    }

    fn opportunities(&self, _limit: usize, _min_score: u32) -> Vec<viz_api::OpportunityDetail> {
        Vec::new()
    }

    fn recent_transactions(&self, _limit: usize) -> Vec<TransactionSummary> {
        Vec::new()
    }

    fn transaction_details(&self, _limit: usize) -> Vec<TransactionDetail> {
        Vec::new()
    }

    fn transaction_detail_by_hash(&self, _hash: &str) -> Option<TransactionDetail> {
        None
    }

    fn market_stats(&self) -> MarketStats {
        MarketStats {
            total_signal_volume: 0,
            total_tx_count: 0,
            low_risk_count: 0,
            medium_risk_count: 0,
            high_risk_count: 0,
            success_rate_bps: 10_000,
        }
    }

    fn metric_snapshot(&self) -> MetricSnapshot {
        MetricSnapshot {
            peer_disconnects_total: 0,
            ingest_lag_ms: 0,
            tx_decode_fail_total: 0,
            tx_decode_total: 1,
            tx_per_sec_current: 0,
            tx_per_sec_baseline: 1,
            storage_write_latency_ms: 0,
            clock_skew_ms: 0,
            queue_depth_current: 0,
            queue_depth_capacity: 1,
        }
    }
}

fn auth_state() -> AppState {
    let mut keys = HashSet::new();
    keys.insert("test-key".to_owned());
    let api_auth = viz_api::auth::ApiAuthConfig {
        enabled: true,
        api_keys: keys,
        requests_per_minute: 60,
    };
    AppState {
        provider: Arc::new(EmptyProvider),
        downsample_limit: 100,
        relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
        alert_thresholds: AlertThresholdConfig::default(),
        api_rate_limiter: viz_api::auth::ApiRateLimiter::new(api_auth.requests_per_minute),
        api_auth,
        live_rpc_chain_status_provider: Arc::new(Vec::<LiveRpcChainStatus>::new),
        live_rpc_drop_metrics_provider: Arc::new(LiveRpcDropMetricsSnapshot::default),
    }
}

#[tokio::test]
async fn protected_routes_require_valid_api_key() {
    let app = build_router(auth_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/transactions")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_routes_allow_valid_api_key() {
    let app = build_router(auth_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/transactions")
                .header("x-api-key", "test-key")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
}
