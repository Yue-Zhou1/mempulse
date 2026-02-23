use common::SourceId;
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{EventStore, InMemoryStorage, NoopClickHouseSink, StorageWal, StorageWriterConfig, spawn_single_writer};

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
