use common::{Address, SourceId};
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use parking_lot::RwLock;
use runtime_core::{
    RuntimeCore, RuntimeCoreConfig, RuntimeCoreDeps, RuntimeCoreStartArgs, RuntimeIngestMode,
};
use scheduler::{SchedulerConfig, ValidatedTransaction, scheduler_channel};
use std::sync::Arc;
use storage::{EventStore, InMemoryStorage, NoopClickHouseSink, spawn_single_writer};
use tokio::time::{Duration, Instant, sleep};
use viz_api::spawn_scheduler_snapshot_writer;

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
async fn scheduler_snapshot_writer_periodically_persists_scheduler_state() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let writer = spawn_single_writer(
        storage.clone(),
        Arc::new(NoopClickHouseSink),
        storage::StorageWriterConfig::default(),
    );
    let (scheduler, runtime) =
        scheduler_channel(SchedulerConfig::default()).expect("valid scheduler config");
    let runtime_task = tokio::spawn(runtime.run());
    let runtime_core = RuntimeCore::start(RuntimeCoreStartArgs {
        deps: RuntimeCoreDeps {
            storage: storage.clone(),
            writer,
            scheduler: scheduler.clone(),
        },
        config: RuntimeCoreConfig {
            ingest_mode: RuntimeIngestMode::Rpc,
            rebuild_scheduler_from_rpc: false,
        },
    })
    .expect("runtime core should start");
    let snapshot_task = spawn_scheduler_snapshot_writer(runtime_core, 10);

    scheduler
        .admit(sample_validated_tx(1, sender(0xa1), 7))
        .await
        .expect("admit tx");

    wait_for(|| {
        storage
            .read()
            .scheduler_snapshot()
            .cloned()
            .is_some_and(|snapshot| snapshot.pending.len() == 1)
    })
    .await;

    snapshot_task.abort();
    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_snapshot_writer_stamps_capture_time_event_watermark() {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let writer = spawn_single_writer(
        storage.clone(),
        Arc::new(NoopClickHouseSink),
        storage::StorageWriterConfig::default(),
    );
    let (scheduler, runtime) =
        scheduler_channel(SchedulerConfig::default()).expect("valid scheduler config");
    let runtime_task = tokio::spawn(runtime.run());
    let runtime_core = RuntimeCore::start(RuntimeCoreStartArgs {
        deps: RuntimeCoreDeps {
            storage: storage.clone(),
            writer,
            scheduler: scheduler.clone(),
        },
        config: RuntimeCoreConfig {
            ingest_mode: RuntimeIngestMode::Rpc,
            rebuild_scheduler_from_rpc: false,
        },
    })
    .expect("runtime core should start");
    let snapshot_task = spawn_scheduler_snapshot_writer(runtime_core, 10);
    let already_durable = sample_validated_tx(1, sender(0xa1), 7);
    let pending = sample_validated_tx(2, sender(0xa1), 8);

    storage
        .write()
        .append_event(decoded_event(1, &already_durable));

    scheduler.admit(pending).await.expect("admit tx");

    wait_for(|| {
        storage
            .read()
            .scheduler_snapshot()
            .cloned()
            .is_some_and(|snapshot| snapshot.pending.len() == 1 && snapshot.event_seq_hi == 1)
    })
    .await;

    snapshot_task.abort();
    runtime_task.abort();
}
