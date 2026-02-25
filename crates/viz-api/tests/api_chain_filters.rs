use axum::{body::Body, http::Request};
use builder::RelayDryRunStatus;
use common::AlertThresholdConfig;
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use std::sync::{Arc, RwLock};
use storage::{
    EventStore, InMemoryStorage, OpportunityRecord, TxFeaturesRecord, TxFullRecord, TxSeenRecord,
};
use tower::util::ServiceExt;
use viz_api::auth::{ApiAuthConfig, ApiRateLimiter};
use viz_api::{
    AppState, DashboardSnapshot, FeatureDetail, InMemoryVizProvider, OpportunityDetail,
    TransactionDetail, TransactionSummary, VizDataProvider, build_router,
};

fn hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn address(seed: u8) -> [u8; 20] {
    [seed; 20]
}

fn decoded_event(seq_id: u64, hash_seed: u8, chain_id: u64) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: 1_700_000_000_000 + seq_id as i64,
        ingest_ts_mono_ns: seq_id * 1_000_000,
        source_id: common::SourceId::new("rpc-test"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash(hash_seed),
            tx_type: 2,
            sender: address(0xaa),
            nonce: seq_id,
            chain_id: Some(chain_id),
            to: Some(address(0xbb)),
            value_wei: Some(1_000),
            gas_limit: Some(210_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(40_000_000_000),
            max_priority_fee_per_gas_wei: Some(3_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        }),
    }
}

fn seed_storage() -> Arc<RwLock<InMemoryStorage>> {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let mut guard = storage.write().expect("lock storage");

    guard.append_event(decoded_event(1, 1, 1));
    guard.append_event(decoded_event(2, 2, 8453));

    for (hash_seed, chain_id) in [(1_u8, 1_u64), (2_u8, 8453_u64)] {
        guard.upsert_tx_seen(TxSeenRecord {
            hash: hash(hash_seed),
            peer: "rpc-ws".to_owned(),
            first_seen_unix_ms: 1_700_000_000_000,
            first_seen_mono_ns: 1_000_000,
            seen_count: 1,
        });
        guard.upsert_tx_full(TxFullRecord {
            hash: hash(hash_seed),
            tx_type: 2,
            sender: address(0xaa),
            nonce: hash_seed as u64,
            to: Some(address(0xbb)),
            chain_id: Some(chain_id),
            value_wei: Some(10_000),
            gas_limit: Some(300_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(40_000_000_000),
            max_priority_fee_per_gas_wei: Some(3_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
            raw_tx: vec![0xde, 0xad, 0xbe, 0xef],
        });
        guard.upsert_tx_features(TxFeaturesRecord {
            hash: hash(hash_seed),
            chain_id: Some(chain_id),
            protocol: if chain_id == 1 {
                "uniswap-v2".to_owned()
            } else {
                "aerodrome".to_owned()
            },
            category: "swap".to_owned(),
            mev_score: if chain_id == 1 { 80 } else { 60 },
            urgency_score: 55,
            method_selector: Some([0x38, 0xed, 0x17, 0x39]),
            feature_engine_version: "feature-engine.v1".to_owned(),
        });
        guard.upsert_opportunity(OpportunityRecord {
            tx_hash: hash(hash_seed),
            strategy: "Sandwich".to_owned(),
            score: if chain_id == 1 { 95 } else { 70 },
            protocol: if chain_id == 1 {
                "uniswap-v2".to_owned()
            } else {
                "aerodrome".to_owned()
            },
            category: "swap".to_owned(),
            chain_id: Some(chain_id),
            feature_engine_version: "feature-engine.v1".to_owned(),
            scorer_version: "scorer.v1".to_owned(),
            strategy_version: "strategy.v1".to_owned(),
            reasons: vec!["test".to_owned()],
            detected_unix_ms: 1_700_000_000_999 + hash_seed as i64,
        });
    }

    drop(guard);
    storage
}

fn build_test_app() -> axum::Router {
    let storage = seed_storage();
    let provider: Arc<dyn VizDataProvider> = Arc::new(InMemoryVizProvider::new(
        storage,
        Arc::new(Vec::new()),
        1,
    ));
    let state = AppState {
        provider,
        downsample_limit: 100,
        relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
        alert_thresholds: AlertThresholdConfig::default(),
        api_auth: ApiAuthConfig::default(),
        api_rate_limiter: ApiRateLimiter::new(600),
    };
    build_router(state)
}

#[tokio::test]
async fn api_chain_filters_apply_to_recent_features_opps_and_transactions() {
    let app = build_test_app();

    let features_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/features/recent?chain_id=1&limit=10")
                .body(Body::empty())
                .expect("features request"),
        )
        .await
        .expect("features response");
    let features_body = axum::body::to_bytes(features_response.into_body(), 1024 * 1024)
        .await
        .expect("features body");
    let features: Vec<FeatureDetail> =
        serde_json::from_slice(&features_body).expect("features payload");
    assert!(features.iter().all(|row| row.chain_id == Some(1)));
    assert!(!features.is_empty());

    let opps_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/opps/recent?chain_id=1&limit=10")
                .body(Body::empty())
                .expect("opps request"),
        )
        .await
        .expect("opps response");
    let opps_body = axum::body::to_bytes(opps_response.into_body(), 1024 * 1024)
        .await
        .expect("opps body");
    let opps: Vec<OpportunityDetail> = serde_json::from_slice(&opps_body).expect("opps payload");
    assert!(opps.iter().all(|row| row.chain_id == Some(1)));
    assert!(!opps.is_empty());

    let tx_all_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/transactions/all?chain_id=1&limit=10")
                .body(Body::empty())
                .expect("transactions/all request"),
        )
        .await
        .expect("transactions/all response");
    let tx_all_body = axum::body::to_bytes(tx_all_response.into_body(), 1024 * 1024)
        .await
        .expect("transactions/all body");
    let tx_all: Vec<TransactionDetail> =
        serde_json::from_slice(&tx_all_body).expect("transactions/all payload");
    assert!(tx_all.iter().all(|row| row.chain_id == Some(1)));
    assert!(!tx_all.is_empty());

    let tx_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/transactions?chain_id=1&limit=10")
                .body(Body::empty())
                .expect("transactions request"),
        )
        .await
        .expect("transactions response");
    let tx_body = axum::body::to_bytes(tx_response.into_body(), 1024 * 1024)
        .await
        .expect("transactions body");
    let tx_rows: Vec<TransactionSummary> =
        serde_json::from_slice(&tx_body).expect("transactions payload");
    assert_eq!(tx_rows.len(), 1);
}

#[tokio::test]
async fn api_chain_filters_apply_to_dashboard_snapshot() {
    let app = build_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/dashboard/snapshot?chain_id=1&tx_limit=10&feature_limit=10&opp_limit=10&replay_limit=100",
                )
                .body(Body::empty())
                .expect("dashboard request"),
        )
        .await
        .expect("dashboard response");

    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("dashboard body");
    let payload: DashboardSnapshot = serde_json::from_slice(&body).expect("dashboard payload");
    assert!(payload.feature_details.iter().all(|row| row.chain_id == Some(1)));
    assert!(payload.opportunities.iter().all(|row| row.chain_id == Some(1)));
    assert_eq!(payload.transactions.len(), 1);
}
