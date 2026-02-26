use common::SourceId;
use event_log::{EventPayload, TxSeen};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use storage::{EventStore, InMemoryStorage, NoopClickHouseSink, StorageWriteOp, StorageWriterConfig, spawn_single_writer};

fn hash(v: u8) -> [u8; 32] {
    [v; 32]
}

#[tokio::test]
async fn append_event_sequence_is_assigned_by_storage_writer() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let sink = Arc::new(NoopClickHouseSink);
    let handle = spawn_single_writer(
        storage.clone(),
        sink,
        StorageWriterConfig {
            queue_capacity: 8,
            flush_batch_size: 1,
            flush_interval_ms: 5,
            wal_path: None,
        },
    );

    handle
        .enqueue(StorageWriteOp::AppendPayload {
            source_id: SourceId::new("rpc-mainnet"),
            payload: EventPayload::TxSeen(TxSeen {
                hash: hash(1),
                peer_id: "peer-a".to_owned(),
                seen_at_unix_ms: 1_700_000_000_000,
                seen_at_mono_ns: 10,
            }),
            ingest_ts_unix_ms: 1_700_000_000_000,
            ingest_ts_mono_ns: 10,
        })
        .await
        .expect("enqueue payload append");

    tokio::time::sleep(Duration::from_millis(30)).await;

    let events = storage.read().expect("storage lock").list_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq_id, 1);
}
