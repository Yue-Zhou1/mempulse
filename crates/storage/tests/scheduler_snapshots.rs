use common::{Address, SourceId};
use event_log::{EventEnvelope, EventPayload, TxConfirmed, TxDecoded};
use scheduler::{
    PersistedSchedulerSnapshot, PersistedSenderQueueEntry, PersistedSenderQueueSnapshot,
    ValidatedTransaction,
};
use std::sync::{Arc, RwLock};
use storage::{
    EventStore, InMemoryStorage, NoopClickHouseSink, StorageWriteOp, StorageWriterConfig,
    spawn_single_writer,
};
use tokio::time::{Duration, sleep};

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

fn confirmed_final_event(seq_id: u64, unix_ms: i64) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: unix_ms,
        ingest_ts_mono_ns: seq_id,
        source_id: SourceId::new("rpc-mainnet"),
        payload: EventPayload::TxConfirmedFinal(TxConfirmed {
            hash: [seq_id as u8; 32],
            block_number: seq_id,
            block_hash: [seq_id as u8; 32],
        }),
    }
}

#[tokio::test]
async fn storage_writer_persists_scheduler_snapshot() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let handle = spawn_single_writer(
        storage.clone(),
        Arc::new(NoopClickHouseSink),
        StorageWriterConfig::default(),
    );
    let ready = sample_validated_tx(1, sender(0xa1), 7);
    let blocked = sample_validated_tx(2, sender(0xa1), 9);
    let snapshot = PersistedSchedulerSnapshot {
        captured_at_unix_ms: 1_700_000_000_321,
        captured_at_mono_ns: 321,
        event_seq_hi: 0,
        pending: vec![ready.clone(), blocked.clone()],
        executable_frontier: vec![ready.hash()],
        sender_queues: vec![PersistedSenderQueueSnapshot {
            sender: sender(0xa1),
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
        }],
    };

    handle
        .enqueue(StorageWriteOp::WriteSchedulerSnapshot(snapshot.clone()))
        .await
        .expect("enqueue snapshot write");
    sleep(Duration::from_millis(25)).await;

    let guard = storage.read().expect("storage readable");
    let persisted = guard
        .scheduler_snapshot()
        .cloned()
        .expect("scheduler snapshot persisted");
    assert_eq!(persisted, snapshot);
}

#[test]
fn storage_rehydration_plan_returns_valid_snapshot_and_tail_events_after_watermark() {
    let mut storage = InMemoryStorage::default();
    let ready = sample_validated_tx(1, sender(0xa1), 7);
    let next = sample_validated_tx(2, sender(0xa1), 8);

    storage.append_event(decoded_event(1, &ready));
    storage.write_scheduler_snapshot(PersistedSchedulerSnapshot {
        captured_at_unix_ms: 1_700_000_000_321,
        captured_at_mono_ns: 321,
        event_seq_hi: 0,
        pending: vec![ready.clone()],
        executable_frontier: vec![ready.hash()],
        sender_queues: vec![PersistedSenderQueueSnapshot {
            sender: sender(0xa1),
            queued: vec![PersistedSenderQueueEntry {
                nonce: 7,
                hash: ready.hash(),
            }],
        }],
    });
    storage.append_event(decoded_event(2, &next));

    let plan = storage.scheduler_rehydration_plan(60_000);
    let snapshot = plan.snapshot.expect("valid snapshot");
    assert_eq!(snapshot.event_seq_hi, 1);
    assert_eq!(
        plan.replay_events
            .into_iter()
            .map(|event| event.seq_id)
            .collect::<Vec<_>>(),
        vec![2]
    );
}

#[test]
fn storage_rehydration_plan_invalidates_snapshot_when_finalized_gap_exceeds_window() {
    let mut storage = InMemoryStorage::default();
    let ready = sample_validated_tx(1, sender(0xa1), 7);

    storage.write_scheduler_snapshot(PersistedSchedulerSnapshot {
        captured_at_unix_ms: 1_700_000_000_000,
        captured_at_mono_ns: 321,
        event_seq_hi: 0,
        pending: vec![ready.clone()],
        executable_frontier: vec![ready.hash()],
        sender_queues: vec![PersistedSenderQueueSnapshot {
            sender: sender(0xa1),
            queued: vec![PersistedSenderQueueEntry {
                nonce: 7,
                hash: ready.hash(),
            }],
        }],
    });
    storage.append_event(confirmed_final_event(1, 1_700_000_000_500));

    let plan = storage.scheduler_rehydration_plan(100);
    assert!(plan.snapshot.is_none());
    assert!(plan.replay_events.is_empty());
    assert!(plan.requires_rpc_rebuild);
}
