use common::{Address, SourceId};
use event_log::TxDecoded;
use scheduler::{
    SchedulerAdmission, SchedulerConfig, SchedulerEnqueueError, ValidatedTransaction,
    scheduler_channel,
};
use tokio::time::{Duration, Instant, sleep};

fn sample_validated_tx(
    hash_seed: u8,
    sender: Address,
    nonce: u64,
    max_fee_per_gas_wei: u128,
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
            sender,
            nonce,
            chain_id: Some(1),
            to: Some([hash_seed.saturating_add(1); 20]),
            value_wei: Some(42),
            gas_limit: Some(21_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(max_fee_per_gas_wei),
            max_priority_fee_per_gas_wei: Some(3),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        },
    }
}

fn sender(seed: u8) -> Address {
    [seed; 20]
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
        max_pending_per_sender: 64,
        replacement_fee_bump_bps: 1_000,
    });

    handle
        .try_admit(sample_validated_tx(
            1,
            sender(1),
            7,
            101,
            "rpc-a",
            1_700_000_000_000,
            10,
        ))
        .expect("first transaction fits in queue");

    let error = handle
        .try_admit(sample_validated_tx(
            2,
            sender(1),
            8,
            101,
            "rpc-a",
            1_700_000_000_001,
            11,
        ))
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

    let tx = sample_validated_tx(9, sender(9), 17, 101, "rpc-mainnet", 1_700_000_123_456, 999);
    let duplicate = tx.clone();
    let later = sample_validated_tx(
        3,
        sender(3),
        19,
        101,
        "rpc-mainnet",
        1_700_000_123_999,
        1_000,
    );

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
    assert_eq!(snapshot.ready.len(), 2);
    assert_eq!(snapshot.blocked.len(), 0);
    assert_eq!(snapshot.sender_queues.len(), 2);
    assert!(snapshot.pending.contains(&later));
    assert!(snapshot.pending.contains(&tx));
    let admitted_tx = snapshot
        .pending
        .iter()
        .find(|candidate| candidate.decoded.hash == tx.decoded.hash)
        .expect("original tx remains pending");
    assert_eq!(admitted_tx.source_id, SourceId::new("rpc-mainnet"));
    assert_eq!(admitted_tx.observed_at_unix_ms, 1_700_000_123_456);
    assert_eq!(admitted_tx.observed_at_mono_ns, 999);

    let metrics = handle.metrics();
    assert_eq!(metrics.queue_full_drop_total, 0);
    assert_eq!(metrics.queue_depth, 0);

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_admit_reports_duplicate_result_to_callers() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let tx = sample_validated_tx(
        90,
        sender(9),
        17,
        101,
        "rpc-mainnet",
        1_700_000_123_456,
        999,
    );

    let first = handle
        .admit(tx.clone())
        .await
        .expect("first admit succeeds");
    let second = handle.admit(tx).await.expect("second admit succeeds");

    assert_eq!(first, SchedulerAdmission::Admitted);
    assert_eq!(second, SchedulerAdmission::Duplicate);

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_admit_reports_replaced_hash_to_callers() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender = sender(0xfa);
    let incumbent = sample_validated_tx(91, sender, 4, 100, "rpc-mainnet", 1_700_000_123_450, 990);
    let replacement =
        sample_validated_tx(92, sender, 4, 110, "rpc-mainnet", 1_700_000_123_451, 991);

    let first = handle
        .admit(incumbent.clone())
        .await
        .expect("incumbent admit succeeds");
    let second = handle
        .admit(replacement)
        .await
        .expect("replacement admit succeeds");

    assert_eq!(first, SchedulerAdmission::Admitted);
    assert_eq!(
        second,
        SchedulerAdmission::Replaced {
            replaced_hash: incumbent.decoded.hash,
        }
    );

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_marks_sender_nonce_gaps_as_blocked_until_gap_is_filled() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender_a = sender(0xa1);
    let sender_b = sender(0xb2);
    let ready = sample_validated_tx(10, sender_a, 7, 100, "rpc-mainnet", 1_700_000_000_010, 10);
    let blocked = sample_validated_tx(11, sender_a, 9, 100, "rpc-mainnet", 1_700_000_000_011, 11);
    let other_sender =
        sample_validated_tx(12, sender_b, 4, 100, "rpc-mainnet", 1_700_000_000_012, 12);

    handle
        .try_admit(ready.clone())
        .expect("enqueue contiguous base nonce");
    handle
        .try_admit(blocked.clone())
        .expect("enqueue gap nonce transaction");
    handle
        .try_admit(other_sender.clone())
        .expect("enqueue second sender transaction");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.pending_total == 3
            && metrics.sender_total == 2
            && metrics.ready_total == 2
            && metrics.blocked_total == 1
    })
    .await;

    let snapshot = handle.snapshot();
    assert_eq!(snapshot.ready.len(), 2);
    assert_eq!(snapshot.blocked, vec![blocked.clone()]);
    assert_eq!(snapshot.sender_queues.len(), 2);
    assert_eq!(snapshot.sender_queues[0].queued.len(), 2);
    assert_eq!(snapshot.sender_queues[0].queued[0], ready);
    assert_eq!(snapshot.sender_queues[0].queued[1], blocked);
    assert_eq!(snapshot.sender_queues[1].queued, vec![other_sender]);

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_replaces_same_nonce_transaction_when_fee_bump_meets_policy() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender = sender(0xcd);
    let incumbent = sample_validated_tx(20, sender, 5, 100, "rpc-mainnet", 1_700_000_001_000, 10);
    let replacement = sample_validated_tx(21, sender, 5, 110, "rpc-mainnet", 1_700_000_001_001, 11);

    handle
        .try_admit(incumbent.clone())
        .expect("enqueue incumbent");
    handle
        .try_admit(replacement.clone())
        .expect("enqueue replacement candidate");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.pending_total == 1
            && metrics.admitted_total == 2
            && metrics.replacement_total == 1
            && metrics.underpriced_replacement_total == 0
    })
    .await;

    let snapshot = handle.snapshot();
    assert_eq!(snapshot.pending, vec![replacement.clone()]);
    assert_eq!(snapshot.ready, vec![replacement]);
    assert_eq!(snapshot.blocked.len(), 0);

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_rejects_underpriced_same_nonce_replacement_and_keeps_incumbent() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender = sender(0xee);
    let incumbent = sample_validated_tx(30, sender, 8, 100, "rpc-mainnet", 1_700_000_002_000, 20);
    let underpriced = sample_validated_tx(31, sender, 8, 109, "rpc-mainnet", 1_700_000_002_001, 21);

    handle
        .try_admit(incumbent.clone())
        .expect("enqueue incumbent");
    handle
        .try_admit(underpriced)
        .expect("enqueue underpriced replacement candidate");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.pending_total == 1
            && metrics.admitted_total == 1
            && metrics.replacement_total == 0
            && metrics.underpriced_replacement_total == 1
    })
    .await;

    let snapshot = handle.snapshot();
    assert_eq!(snapshot.pending, vec![incumbent.clone()]);
    assert_eq!(snapshot.ready, vec![incumbent]);

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_drops_new_nonce_when_sender_queue_capacity_is_exceeded() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig {
        handoff_queue_capacity: 64,
        max_pending_per_sender: 2,
        replacement_fee_bump_bps: 1_000,
    });
    let runtime_task = tokio::spawn(runtime.run());

    let sender = sender(0xfa);
    handle
        .try_admit(sample_validated_tx(
            40,
            sender,
            1,
            100,
            "rpc-mainnet",
            1_700_000_003_000,
            30,
        ))
        .expect("enqueue nonce 1");
    handle
        .try_admit(sample_validated_tx(
            41,
            sender,
            2,
            100,
            "rpc-mainnet",
            1_700_000_003_001,
            31,
        ))
        .expect("enqueue nonce 2");
    handle
        .try_admit(sample_validated_tx(
            42,
            sender,
            3,
            100,
            "rpc-mainnet",
            1_700_000_003_002,
            32,
        ))
        .expect("enqueue nonce 3 candidate");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.pending_total == 2 && metrics.sender_limit_drop_total == 1
    })
    .await;

    let snapshot = handle.snapshot();
    assert_eq!(snapshot.pending.len(), 2);
    assert_eq!(snapshot.sender_queues[0].queued.len(), 2);
    assert_eq!(snapshot.sender_queues[0].queued[0].decoded.nonce, 1);
    assert_eq!(snapshot.sender_queues[0].queued[1].decoded.nonce, 2);

    runtime_task.abort();
}
