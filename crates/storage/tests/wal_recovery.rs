use common::SourceId;
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{
    EventStore, InMemoryStorage, NoopClickHouseSink, StorageWal, StorageWriterConfig,
    spawn_single_writer,
};

fn hash(v: u8) -> [u8; 32] {
    [v; 32]
}

fn temp_wal_path(suffix: &str) -> std::path::PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("prototype03-storage-wal-{suffix}-{now}.log"))
}

fn decoded_event(seq: u64, seed: u8) -> EventEnvelope {
    EventEnvelope {
        seq_id: seq,
        ingest_ts_unix_ms: 1_700_000_000_000 + seq as i64,
        ingest_ts_mono_ns: seq * 10,
        source_id: SourceId::new("wal-test"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash(seed),
            tx_type: 2,
            sender: [seed; 20],
            nonce: seq,
            chain_id: Some(1),
            to: None,
            value_wei: None,
            gas_limit: Some(90_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(30_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(12),
        }),
    }
}

#[tokio::test]
async fn writer_recovers_unflushed_events_from_wal_on_restart() {
    let wal_path = temp_wal_path("recovery");
    let wal = StorageWal::new(&wal_path).expect("create wal");
    wal.append_event(&decoded_event(1, 9))
        .expect("append event to wal");
    let recovered = wal.recover_events().expect("recover from wal directly");
    assert_eq!(recovered.len(), 1);

    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let sink = Arc::new(NoopClickHouseSink);
    let _handle = spawn_single_writer(
        storage.clone(),
        sink,
        StorageWriterConfig {
            queue_capacity: 8,
            flush_batch_size: 8,
            flush_interval_ms: 1_000,
            wal_path: Some(wal_path.clone()),
        },
    );

    tokio::time::sleep(Duration::from_millis(40)).await;
    let events = storage.read().expect("storage lock").list_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq_id, 1);

    let _ = std::fs::remove_file(wal_path);
}

#[test]
fn wal_scan_returns_events_after_seq_with_limit() {
    let wal_path = temp_wal_path("scan");
    let wal = StorageWal::new(&wal_path).expect("create wal");
    wal.append_event(&decoded_event(1, 1))
        .expect("append event 1 to wal");
    wal.append_event(&decoded_event(2, 2))
        .expect("append event 2 to wal");
    wal.append_event(&decoded_event(3, 3))
        .expect("append event 3 to wal");

    let scanned = wal.scan(1, 1).expect("scan wal");
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].seq_id, 2);

    let _ = std::fs::remove_file(wal_path);
}

#[test]
fn wal_segments_roll_over_and_scan_across_segments() {
    let wal_path = temp_wal_path("segments");
    let wal = StorageWal::with_segment_size(&wal_path, 256).expect("create segmented wal");

    for seq in 1_u64..=12_u64 {
        wal.append_event(&decoded_event(seq, seq as u8))
            .expect("append event to segmented wal");
    }

    let recovered = wal.recover_events().expect("recover segmented wal events");
    assert_eq!(recovered.len(), 12);
    assert_eq!(recovered.first().map(|event| event.seq_id), Some(1));
    assert_eq!(recovered.last().map(|event| event.seq_id), Some(12));

    let scanned = wal.scan(6, 4).expect("scan segmented wal");
    assert_eq!(
        scanned.into_iter().map(|event| event.seq_id).collect::<Vec<_>>(),
        vec![7, 8, 9, 10]
    );

    let base_name = wal
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("wal base filename");
    let segment_prefix = format!("{base_name}.seg.");
    let parent = wal.path().parent().expect("wal parent directory");
    let segment_count = std::fs::read_dir(parent)
        .expect("list wal parent")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&segment_prefix))
        })
        .count();
    assert!(
        segment_count >= 2,
        "expected multiple WAL segments, got {segment_count}"
    );

    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(&segment_prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    let _ = std::fs::remove_file(wal_path);
}
