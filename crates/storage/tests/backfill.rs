use common::{SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxSeen};
use std::sync::{Arc, RwLock};
use storage::{BackfillConfig, BackfillSummary, BackfillWriter, EventStore, InMemoryStorage};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn seen_event(seq_id: u64, ts_unix_ms: i64, hash_v: u8) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: ts_unix_ms,
        ingest_ts_mono_ns: seq_id * 10,
        source_id: SourceId::new("backfill-test"),
        payload: EventPayload::TxSeen(TxSeen {
            hash: hash(hash_v),
            peer_id: "peer-a".to_owned(),
            seen_at_unix_ms: ts_unix_ms,
            seen_at_mono_ns: seq_id * 10,
        }),
    }
}

#[test]
fn backfill_is_idempotent_for_duplicate_event_windows() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let mut writer = BackfillWriter::new(
        storage.clone(),
        BackfillConfig {
            retention_window_ms: 10 * 60 * 1000,
        },
    );
    let now = 1_710_000_000_000_i64;
    let window = vec![
        seen_event(1, now - 1_000, 1),
        seen_event(2, now - 900, 2),
        seen_event(3, now - 800, 3),
    ];

    let first: BackfillSummary = writer.apply_events(&window, now);
    let second: BackfillSummary = writer.apply_events(&window, now);

    assert_eq!(first.inserted, 3);
    assert_eq!(first.skipped_duplicates, 0);
    assert_eq!(second.inserted, 0);
    assert_eq!(second.skipped_duplicates, 3);
    assert_eq!(storage.read().unwrap().list_events().len(), 3);
}

#[test]
fn backfill_prunes_events_outside_retention_window() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let mut writer = BackfillWriter::new(
        storage.clone(),
        BackfillConfig {
            retention_window_ms: 2_000,
        },
    );
    let now = 1_710_000_000_000_i64;
    let window = vec![
        seen_event(1, now - 4_000, 1),
        seen_event(2, now - 1_500, 2),
        seen_event(3, now - 500, 3),
    ];

    let summary = writer.apply_events(&window, now);
    let events = storage.read().unwrap().list_events();

    assert_eq!(summary.inserted, 2);
    assert_eq!(summary.pruned_by_retention, 1);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].seq_id, 2);
    assert_eq!(events[1].seq_id, 3);
}
