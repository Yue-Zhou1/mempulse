use common::{Address, SourceId, TxHash};
use event_log::TxDecoded;
use scheduler::{
    PersistedSchedulerSnapshot, PersistedSenderQueueEntry, PersistedSenderQueueSnapshot,
    SchedulerConfig, ValidatedTransaction, scheduler_channel, scheduler_channel_with_snapshot,
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

fn hashes(txs: &[ValidatedTransaction]) -> Vec<TxHash> {
    txs.iter().map(ValidatedTransaction::hash).collect()
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

#[tokio::test]
async fn scheduler_persisted_snapshot_captures_frontier_and_rehydrates_state() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender_a = sender(0xa1);
    let sender_b = sender(0xb2);
    let ready = sample_validated_tx(10, sender_a, 7, 100, "rpc-mainnet", 1_700_000_000_010, 10);
    let blocked = sample_validated_tx(11, sender_a, 9, 100, "rpc-mainnet", 1_700_000_000_011, 11);
    let other_sender =
        sample_validated_tx(12, sender_b, 4, 100, "rpc-mainnet", 1_700_000_000_012, 12);

    handle.try_admit(ready.clone()).expect("enqueue ready tx");
    handle
        .try_admit(blocked.clone())
        .expect("enqueue blocked tx");
    handle
        .try_admit(other_sender.clone())
        .expect("enqueue second sender tx");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.pending_total == 3 && metrics.ready_total == 2 && metrics.blocked_total == 1
    })
    .await;

    let before = handle.snapshot();
    let persisted = handle.persisted_snapshot(1_700_000_000_099, 99);

    assert_eq!(
        persisted,
        PersistedSchedulerSnapshot {
            captured_at_unix_ms: 1_700_000_000_099,
            captured_at_mono_ns: 99,
            event_seq_hi: 0,
            pending: vec![ready.clone(), blocked.clone(), other_sender.clone()],
            executable_frontier: vec![ready.hash(), other_sender.hash()],
            sender_queues: vec![
                PersistedSenderQueueSnapshot {
                    sender: sender_a,
                    queued: vec![
                        PersistedSenderQueueEntry {
                            nonce: 7,
                            hash: ready.hash(),
                        },
                        PersistedSenderQueueEntry {
                            nonce: 9,
                            hash: blocked.hash(),
                        },
                    ],
                },
                PersistedSenderQueueSnapshot {
                    sender: sender_b,
                    queued: vec![PersistedSenderQueueEntry {
                        nonce: 4,
                        hash: other_sender.hash(),
                    }],
                },
            ],
        }
    );

    runtime_task.abort();

    let (restored_handle, restored_runtime) =
        scheduler_channel_with_snapshot(SchedulerConfig::default(), persisted)
            .expect("rehydrate scheduler from persisted snapshot");
    let restored_runtime_task = tokio::spawn(restored_runtime.run());

    wait_for(|| {
        let metrics = restored_handle.metrics();
        metrics.pending_total == 3 && metrics.ready_total == 2 && metrics.blocked_total == 1
    })
    .await;

    let after = restored_handle.snapshot();
    assert_eq!(hashes(&after.pending), hashes(&before.pending));
    assert_eq!(hashes(&after.ready), hashes(&before.ready));
    assert_eq!(hashes(&after.blocked), hashes(&before.blocked));
    assert_eq!(after.sender_queues, before.sender_queues);

    restored_runtime_task.abort();
}

#[tokio::test]
async fn scheduler_get_pending_transactions_returns_requested_hashes_in_input_order() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let sender_a = sender(0xa1);
    let sender_b = sender(0xb2);
    let ready = sample_validated_tx(10, sender_a, 7, 100, "rpc-mainnet", 1_700_000_000_010, 10);
    let blocked = sample_validated_tx(11, sender_a, 9, 100, "rpc-mainnet", 1_700_000_000_011, 11);
    let other_sender =
        sample_validated_tx(12, sender_b, 4, 100, "rpc-mainnet", 1_700_000_000_012, 12);

    handle.try_admit(ready.clone()).expect("enqueue ready tx");
    handle
        .try_admit(blocked.clone())
        .expect("enqueue blocked tx");
    handle
        .try_admit(other_sender.clone())
        .expect("enqueue second sender tx");

    wait_for(|| {
        let metrics = handle.metrics();
        metrics.pending_total == 3
    })
    .await;

    let pending = handle.get_pending_transactions(&[
        other_sender.hash(),
        [0xff; 32],
        ready.hash(),
        ready.hash(),
    ]);

    assert_eq!(pending, vec![other_sender, ready.clone(), ready]);

    runtime_task.abort();
}
