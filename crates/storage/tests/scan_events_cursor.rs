use common::SourceId;
use event_log::{EventEnvelope, EventPayload, TxSeen};
use storage::{EventStore, InMemoryStorage, StorageConfig, scan_events_cursor_start};

fn hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn seen_event(seq_id: u64, hash_seed: u8) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: 1_700_000_000_000 + seq_id as i64,
        ingest_ts_mono_ns: seq_id * 1_000_000,
        source_id: SourceId::new("test-source"),
        payload: EventPayload::TxSeen(TxSeen {
            hash: hash(hash_seed),
            peer_id: "peer-a".to_owned(),
            seen_at_unix_ms: 1_700_000_000_000 + seq_id as i64,
            seen_at_mono_ns: seq_id * 1_000_000,
        }),
    }
}

#[test]
fn scan_events_cursor_start_jumps_to_tail_window() {
    let events = (1..=10_000)
        .map(|seq| seen_event(seq, (seq % u8::MAX as u64) as u8))
        .collect::<Vec<_>>();

    let start = scan_events_cursor_start(&events, 9_900);
    assert_eq!(start, 9_900);
}

#[test]
fn scan_events_cursor_returns_deterministic_window_after_seq() {
    let mut storage = InMemoryStorage::default();
    storage.append_event(seen_event(5, 5));
    storage.append_event(seen_event(3, 3));
    storage.append_event(seen_event(4, 4));

    let seqs = storage
        .scan_events(3, 2)
        .into_iter()
        .map(|event| event.seq_id)
        .collect::<Vec<_>>();

    assert_eq!(seqs, vec![4, 5]);
}

#[test]
fn scan_events_cursor_scales_for_large_event_buffers() {
    let mut storage = InMemoryStorage::with_config(StorageConfig {
        event_capacity: 100_000,
        recent_tx_capacity: 100_000,
        ..StorageConfig::default()
    });

    for seq in 1..=50_000 {
        storage.append_event(seen_event(seq, (seq % u8::MAX as u64) as u8));
    }

    let scan = storage.scan_events(49_000, 32);
    assert_eq!(scan.len(), 32);
    assert_eq!(scan.first().map(|event| event.seq_id), Some(49_001));
    assert_eq!(scan.last().map(|event| event.seq_id), Some(49_032));
}
