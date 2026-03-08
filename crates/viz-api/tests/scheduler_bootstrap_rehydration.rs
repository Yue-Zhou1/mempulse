use common::{Address, SourceId};
use event_log::{EventEnvelope, EventPayload, TxConfirmed, TxDecoded, TxReplaced};
use scheduler::{
    PersistedSchedulerSnapshot, PersistedSenderQueueEntry, PersistedSenderQueueSnapshot,
    ValidatedTransaction,
};
use std::sync::{Arc, RwLock};
use storage::{EventStore, InMemoryStorage};
use tokio::time::{Duration, Instant, sleep};
use viz_api::{
    SchedulerRehydrationConfig, default_state_with_runtime_from_storage,
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

fn confirmed_final_event(seq_id: u64, tx: &ValidatedTransaction) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: tx.observed_at_unix_ms + 1_000,
        ingest_ts_mono_ns: tx.observed_at_mono_ns + 1_000,
        source_id: tx.source_id.clone(),
        payload: EventPayload::TxConfirmedFinal(TxConfirmed {
            hash: tx.hash(),
            block_number: 100 + seq_id,
            block_hash: [seq_id as u8; 32],
        }),
    }
}

fn replaced_event(
    seq_id: u64,
    replaced: &ValidatedTransaction,
    replacement: &ValidatedTransaction,
) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: replacement.observed_at_unix_ms,
        ingest_ts_mono_ns: replacement.observed_at_mono_ns,
        source_id: replacement.source_id.clone(),
        payload: EventPayload::TxReplaced(TxReplaced {
            hash: replaced.hash(),
            replaced_by: replacement.hash(),
        }),
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

#[tokio::test]
async fn binary_bootstrap_rehydrates_scheduler_from_storage_snapshot() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let ready = sample_validated_tx(1, sender(0xa1), 7);
    let blocked = sample_validated_tx(2, sender(0xa1), 9);

    storage
        .write()
        .expect("storage writable")
        .write_scheduler_snapshot(PersistedSchedulerSnapshot {
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
        });

    let (_state, bootstrap) = default_state_with_runtime_from_storage(storage);
    let snapshot = bootstrap.scheduler.snapshot();

    assert_eq!(snapshot.pending, vec![ready.clone(), blocked.clone()]);
    assert_eq!(snapshot.ready, vec![ready]);
    assert_eq!(snapshot.blocked, vec![blocked]);
}

#[tokio::test]
async fn binary_bootstrap_replays_post_snapshot_decoded_tail_events() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let ready = sample_validated_tx(1, sender(0xa1), 7);
    let tail = sample_validated_tx(2, sender(0xa1), 8);

    {
        let mut guard = storage.write().expect("storage writable");
        guard.append_event(decoded_event(1, &ready));
        guard.write_scheduler_snapshot(PersistedSchedulerSnapshot {
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
        guard.append_event(decoded_event(2, &tail));
    }

    let (_state, bootstrap) = default_state_with_runtime_from_storage(storage);
    let snapshot = bootstrap.scheduler.snapshot();

    assert_eq!(snapshot.pending, vec![ready.clone(), tail.clone()]);
    assert_eq!(snapshot.ready, vec![ready, tail]);
    assert!(snapshot.blocked.is_empty());
}

#[tokio::test]
async fn binary_bootstrap_prunes_snapshot_transactions_confirmed_in_wal_tail() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let ready = sample_validated_tx(1, sender(0xa1), 7);

    {
        let mut guard = storage.write().expect("storage writable");
        guard.append_event(decoded_event(1, &ready));
        guard.write_scheduler_snapshot(PersistedSchedulerSnapshot {
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
        guard.append_event(confirmed_final_event(2, &ready));
    }

    let (_state, bootstrap) = default_state_with_runtime_from_storage(storage);
    let snapshot = bootstrap.scheduler.snapshot();

    assert!(snapshot.pending.is_empty());
    assert!(snapshot.ready.is_empty());
    assert!(snapshot.blocked.is_empty());
}

#[tokio::test]
async fn binary_bootstrap_prunes_snapshot_transactions_replaced_in_wal_tail() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let replaced = sample_validated_tx(1, sender(0xa1), 7);
    let replacement = sample_validated_tx(2, sender(0xa1), 7);

    {
        let mut guard = storage.write().expect("storage writable");
        guard.append_event(decoded_event(1, &replaced));
        guard.write_scheduler_snapshot(PersistedSchedulerSnapshot {
            captured_at_unix_ms: 1_700_000_000_321,
            captured_at_mono_ns: 321,
            event_seq_hi: 0,
            pending: vec![replaced.clone()],
            executable_frontier: vec![replaced.hash()],
            sender_queues: vec![PersistedSenderQueueSnapshot {
                sender: sender(0xa1),
                queued: vec![PersistedSenderQueueEntry {
                    nonce: 7,
                    hash: replaced.hash(),
                }],
            }],
        });
        guard.append_event(decoded_event(2, &replacement));
        guard.append_event(replaced_event(3, &replaced, &replacement));
    }

    let (_state, bootstrap) = default_state_with_runtime_from_storage(storage);
    let snapshot = bootstrap.scheduler.snapshot();

    assert_eq!(snapshot.pending, vec![replacement.clone()]);
    assert_eq!(snapshot.ready, vec![replacement]);
    assert!(snapshot.blocked.is_empty());
}

#[tokio::test]
async fn binary_bootstrap_ignores_snapshot_when_finalized_gap_is_stale() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let ready = sample_validated_tx(1, sender(0xa1), 7);

    storage
        .write()
        .expect("storage writable")
        .write_scheduler_snapshot(PersistedSchedulerSnapshot {
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

    {
        let mut guard = storage.write().expect("storage writable");
        guard.append_event(EventEnvelope {
            seq_id: 1,
            ingest_ts_unix_ms: 1_700_000_000_500,
            ingest_ts_mono_ns: 1,
            source_id: SourceId::new("rpc-mainnet"),
            payload: EventPayload::TxConfirmedFinal(event_log::TxConfirmed {
                hash: [0x99; 32],
                block_number: 100,
                block_hash: [0x42; 32],
            }),
        });
    }

    let (_state, bootstrap) = default_state_with_runtime_from_storage_and_rehydration(
        storage,
        SchedulerRehydrationConfig {
            snapshot_interval_ms: 5_000,
            snapshot_max_finality_age_ms: 100,
        },
    );

    assert!(bootstrap.scheduler.snapshot().pending.is_empty());
    assert!(bootstrap.rebuild_scheduler_from_rpc);
}

#[tokio::test]
async fn binary_bootstrap_exposes_snapshot_writer_shutdown_handle() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let (_state, bootstrap) = default_state_with_runtime_from_storage_and_rehydration(
        storage.clone(),
        SchedulerRehydrationConfig {
            snapshot_interval_ms: 10,
            snapshot_max_finality_age_ms: 60_000,
        },
    );

    wait_for(|| {
        storage
            .read()
            .ok()
            .and_then(|guard| guard.scheduler_snapshot().cloned())
            .is_some()
    })
    .await;

    bootstrap.abort_background_tasks();

    bootstrap
        .scheduler
        .admit(sample_validated_tx(3, sender(0xb2), 1))
        .await
        .expect("admit tx after abort");

    sleep(Duration::from_millis(50)).await;

    let persisted = storage
        .read()
        .expect("storage readable")
        .scheduler_snapshot()
        .cloned()
        .expect("scheduler snapshot persisted");
    assert!(persisted.pending.is_empty());
}
