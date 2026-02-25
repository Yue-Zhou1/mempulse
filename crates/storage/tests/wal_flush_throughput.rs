use anyhow::Result;
use async_trait::async_trait;
use common::SourceId;
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{
    ClickHouseBatchSink, EventStore, InMemoryStorage, StorageWal, StorageWriteOp,
    StorageWriterConfig, spawn_single_writer,
};

fn hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn decoded_event(seq: u64, seed: u8) -> EventEnvelope {
    EventEnvelope {
        seq_id: seq,
        ingest_ts_unix_ms: 1_700_000_000_000 + seq as i64,
        ingest_ts_mono_ns: seq.saturating_mul(1_000_000),
        source_id: SourceId::new("throughput-test"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash(seed),
            tx_type: 2,
            sender: [0x11; 20],
            nonce: seq,
            chain_id: Some(1),
            to: Some([0x22; 20]),
            value_wei: Some(1_000),
            gas_limit: Some(300_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(40_000_000_000),
            max_priority_fee_per_gas_wei: Some(3_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        }),
    }
}

fn temp_wal_path(suffix: &str) -> std::path::PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("prototype03-wal-throughput-{suffix}-{now}.log"))
}

#[derive(Clone)]
struct RecordingSink {
    flush_sizes: Arc<RwLock<Vec<usize>>>,
}

#[async_trait]
impl ClickHouseBatchSink for RecordingSink {
    async fn flush_event_batch(&self, events: Vec<EventEnvelope>) -> Result<()> {
        self.flush_sizes
            .write()
            .expect("lock flush sizes")
            .push(events.len());
        Ok(())
    }
}

#[test]
fn wal_flush_throughput_exposes_high_volume_tuning_presets() {
    let writer = StorageWriterConfig::high_throughput_defaults();
    assert!(writer.flush_batch_size > StorageWriterConfig::default().flush_batch_size);
    assert!(writer.flush_interval_ms < StorageWriterConfig::default().flush_interval_ms);

    let wal =
        StorageWal::high_throughput(temp_wal_path("preset")).expect("create high throughput wal");
    assert!(wal.segment_max_bytes() > 64 * 1024 * 1024);
}

#[tokio::test]
async fn wal_flush_throughput_keeps_large_batches_and_clears_wal_under_high_rate() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let flush_sizes = Arc::new(RwLock::new(Vec::new()));
    let sink = Arc::new(RecordingSink {
        flush_sizes: flush_sizes.clone(),
    });
    let wal_path = temp_wal_path("high-rate");
    let wal = StorageWal::high_throughput(&wal_path).expect("create throughput wal");

    let mut writer_config = StorageWriterConfig::high_throughput_defaults();
    writer_config.flush_batch_size = 64;
    writer_config.flush_interval_ms = 20;
    writer_config.queue_capacity = 2_048;
    writer_config.wal_path = Some(wal_path.clone());
    let handle = spawn_single_writer(storage.clone(), sink, writer_config);

    for seq in 1..=256_u64 {
        handle
            .enqueue(StorageWriteOp::AppendEvent(decoded_event(
                seq,
                (seq % 255) as u8,
            )))
            .await
            .expect("enqueue append event");
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    let sizes = flush_sizes
        .read()
        .expect("lock flush sizes")
        .iter()
        .copied()
        .collect::<Vec<_>>();
    assert!(!sizes.is_empty());
    assert!(sizes.iter().copied().max().unwrap_or(0) >= 64);

    let recovered = wal.recover_events().expect("recover wal after flush");
    assert!(
        recovered.is_empty(),
        "wal should be cleared after successful flush"
    );

    let events = storage.read().expect("lock storage").list_events();
    assert_eq!(events.len(), 256);

    let _ = std::fs::remove_file(wal_path);
}
