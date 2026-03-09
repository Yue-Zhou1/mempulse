use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{Address, SourceId};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxReorged};
use scheduler::{PersistedSchedulerSnapshot, ValidatedTransaction};
use std::sync::{Arc, RwLock};
use storage::{EventStore, InMemoryStorage, TxFullRecord};
use tokio::time::{Duration, Instant, sleep};
use tower::util::ServiceExt;
use viz_api::{
    SchedulerRehydrationConfig, build_router, default_state_with_runtime_from_storage,
    default_state_with_runtime_from_storage_and_rehydration,
};

fn sender(seed: u8) -> Address {
    [seed; 20]
}

fn sample_validated_tx(hash_seed: u8, sender: Address, nonce: u64) -> ValidatedTransaction {
    ValidatedTransaction {
        source_id: SourceId::new("rpc-mainnet"),
        observed_at_unix_ms: 1_700_000_000_000 + hash_seed as i64,
        observed_at_mono_ns: hash_seed as u64,
        calldata: vec![hash_seed; 4],
        decoded: TxDecoded {
            hash: [hash_seed; 32],
            tx_type: 2,
            sender,
            nonce,
            chain_id: Some(1),
            to: Some([hash_seed.saturating_add(1); 20]),
            value_wei: Some(42),
            gas_limit: Some(21_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(100),
            max_priority_fee_per_gas_wei: Some(3),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        },
    }
}

fn decoded_event(seq_id: u64, tx: &ValidatedTransaction) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: tx.observed_at_unix_ms,
        ingest_ts_mono_ns: tx.observed_at_mono_ns,
        source_id: tx.source_id.clone(),
        payload: EventPayload::TxDecoded(tx.decoded.clone()),
    }
}

fn reorg_event(seq_id: u64, tx: &ValidatedTransaction) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: tx.observed_at_unix_ms + 1_000,
        ingest_ts_mono_ns: tx.observed_at_mono_ns + 1_000,
        source_id: tx.source_id.clone(),
        payload: EventPayload::TxReorged(TxReorged {
            hash: tx.hash(),
            old_block_hash: [0x44; 32],
            new_block_hash: [0x55; 32],
        }),
    }
}

fn tx_full_record(tx: &ValidatedTransaction) -> TxFullRecord {
    TxFullRecord {
        hash: tx.hash(),
        tx_type: tx.decoded.tx_type,
        sender: tx.decoded.sender,
        nonce: tx.decoded.nonce,
        to: tx.decoded.to,
        chain_id: tx.decoded.chain_id,
        value_wei: tx.decoded.value_wei,
        gas_limit: tx.decoded.gas_limit,
        gas_price_wei: tx.decoded.gas_price_wei,
        max_fee_per_gas_wei: tx.decoded.max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei: tx.decoded.max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei: tx.decoded.max_fee_per_blob_gas_wei,
        calldata_len: Some(tx.calldata.len() as u32),
        raw_tx: tx.calldata.clone(),
    }
}

#[tokio::test]
async fn metrics_route_reports_replay_lag_checkpoint_duration_and_reorg_tail_depth() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let tx = sample_validated_tx(1, sender(0xa1), 7);

    {
        let mut guard = storage.write().expect("storage writable");
        guard.upsert_tx_full(tx_full_record(&tx));
        guard.append_event(decoded_event(1, &tx));
        guard.write_scheduler_snapshot(PersistedSchedulerSnapshot {
            captured_at_unix_ms: 1_700_000_000_321,
            captured_at_mono_ns: 321,
            event_seq_hi: 0,
            pending: vec![tx.clone()],
            executable_frontier: vec![tx.hash()],
            sender_queues: Vec::new(),
        });
        guard.append_event(reorg_event(2, &tx));
    }

    let (state, _bootstrap) = default_state_with_runtime_from_storage(storage);
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

    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read metrics payload");
    let payload = String::from_utf8(body.to_vec()).expect("metrics payload is utf8");

    assert!(payload.contains("mempulse_replay_lag_events 1"));
    assert!(payload.contains("mempulse_replay_tail_reorged_tx_total 1"));
    assert!(payload.contains("mempulse_replay_reorg_depth 1"));
    assert!(payload.contains("mempulse_replay_checkpoint_duration_ms "));
    assert!(!payload.contains("mempulse_replay_checkpoint_duration_ms 0"));
}

async fn wait_for_metric(app: &axum::Router, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("build metrics request"),
            )
            .await
            .expect("run metrics request");
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read metrics payload");
        let payload = String::from_utf8(body.to_vec()).expect("metrics payload is utf8");
        if payload.contains(needle) {
            return payload;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("metric `{needle}` not observed before timeout");
}

#[tokio::test]
async fn metrics_route_serves_cached_replay_metrics_until_background_refresh() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let tx = sample_validated_tx(2, sender(0xb2), 8);

    {
        let mut guard = storage.write().expect("storage writable");
        guard.upsert_tx_full(tx_full_record(&tx));
        guard.write_scheduler_snapshot(PersistedSchedulerSnapshot {
            captured_at_unix_ms: 1_700_000_000_500,
            captured_at_mono_ns: 500,
            event_seq_hi: 0,
            pending: vec![tx.clone()],
            executable_frontier: vec![tx.hash()],
            sender_queues: Vec::new(),
        });
    }

    let (state, bootstrap) = default_state_with_runtime_from_storage_and_rehydration(
        storage.clone(),
        SchedulerRehydrationConfig {
            snapshot_interval_ms: 25,
            snapshot_max_finality_age_ms: 300_000,
        },
    );
    let app = build_router(state);

    let initial_payload = wait_for_metric(
        &app,
        "mempulse_replay_reorg_depth 0",
        Duration::from_secs(1),
    )
    .await;
    assert!(initial_payload.contains("mempulse_replay_lag_events 0"));
    assert!(initial_payload.contains("mempulse_replay_tail_reorged_tx_total 0"));

    {
        let mut guard = storage.write().expect("storage writable");
        guard.append_event(reorg_event(1, &tx));
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("build metrics request"),
        )
        .await
        .expect("run metrics request");
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read metrics payload");
    let payload = String::from_utf8(body.to_vec()).expect("metrics payload is utf8");
    assert!(
        payload.contains("mempulse_replay_reorg_depth 0"),
        "expected cached replay metrics before refresh, got: {payload}"
    );

    let refreshed_payload = wait_for_metric(
        &app,
        "mempulse_replay_reorg_depth 1",
        Duration::from_secs(1),
    )
    .await;
    assert!(refreshed_payload.contains("mempulse_replay_lag_events 1"));
    assert!(refreshed_payload.contains("mempulse_replay_tail_reorged_tx_total 1"));

    bootstrap.abort_background_tasks();
}
