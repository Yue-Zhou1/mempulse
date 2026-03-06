use common::SourceId;
use event_log::TxDecoded;
use scheduler::{SchedulerConfig, SchedulerEnqueueError, ValidatedTransaction, scheduler_channel};
use tokio::time::{Duration, Instant, sleep};

fn sample_validated_tx(
    hash_seed: u8,
    nonce: u64,
    source: &str,
    observed_at_unix_ms: i64,
    observed_at_mono_ns: u64,
) -> ValidatedTransaction {
    ValidatedTransaction {
        source_id: SourceId::new(source),
        observed_at_unix_ms,
        observed_at_mono_ns,
        decoded: TxDecoded {
            hash: [hash_seed; 32],
            tx_type: 2,
            sender: [hash_seed; 20],
            nonce,
            chain_id: Some(1),
            to: Some([hash_seed.saturating_add(1); 20]),
            value_wei: Some(42),
            gas_limit: Some(21_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(101),
            max_priority_fee_per_gas_wei: Some(3),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        },
    }
}

async fn wait_for(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met before timeout");
}

#[test]
fn scheduler_handoff_queue_drops_new_transactions_when_full() {
    let (handle, _runtime) = scheduler_channel(SchedulerConfig {
        handoff_queue_capacity: 1,
    });

    handle
        .try_admit(sample_validated_tx(1, 7, "rpc-a", 1_700_000_000_000, 10))
        .expect("first transaction fits in queue");

    let error = handle
        .try_admit(sample_validated_tx(2, 8, "rpc-a", 1_700_000_000_001, 11))
        .expect_err("second transaction should be dropped before the runtime drains it");

    assert_eq!(error, SchedulerEnqueueError::QueueFull);

    let metrics = handle.metrics();
    assert_eq!(metrics.handoff_queue_capacity, 1);
    assert_eq!(metrics.queue_depth, 1);
    assert_eq!(metrics.queue_full_drop_total, 1);
    assert_eq!(metrics.pending_total, 0);
}

#[tokio::test]
async fn scheduler_runtime_admits_unique_transactions_and_deduplicates_by_hash() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let tx = sample_validated_tx(9, 17, "rpc-mainnet", 1_700_000_123_456, 999);
    let duplicate = tx.clone();
    let later = sample_validated_tx(3, 19, "rpc-mainnet", 1_700_000_123_999, 1_000);

    handle.try_admit(tx.clone()).expect("enqueue first tx");
    handle
        .try_admit(duplicate)
        .expect("enqueue duplicate tx for dedup accounting");
    handle
        .try_admit(later.clone())
        .expect("enqueue second unique tx");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.admitted_total == 2 && metrics.duplicate_total == 1 && metrics.pending_total == 2
    })
    .await;

    let snapshot = handle.snapshot();
    assert_eq!(snapshot.pending.len(), 2);
    assert_eq!(snapshot.pending[0].decoded.hash, later.decoded.hash);
    assert_eq!(snapshot.pending[1], tx);
    assert_eq!(snapshot.pending[1].source_id, SourceId::new("rpc-mainnet"));
    assert_eq!(snapshot.pending[1].observed_at_unix_ms, 1_700_000_123_456);
    assert_eq!(snapshot.pending[1].observed_at_mono_ns, 999);

    let metrics = handle.metrics();
    assert_eq!(metrics.queue_full_drop_total, 0);
    assert_eq!(metrics.queue_depth, 0);

    runtime_task.abort();
}
