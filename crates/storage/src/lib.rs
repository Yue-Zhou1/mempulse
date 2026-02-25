mod backfill;
mod clickhouse_schema;
mod wal;

use ahash::RandomState;
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use common::{Address, PeerId, TxHash};
use event_log::{EventEnvelope, EventPayload, GlobalSequencer, cmp_deterministic};
use hashbrown::{HashMap, HashSet};
use replay::ReplayFrame;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::time::MissedTickBehavior;

pub use backfill::{BackfillConfig, BackfillSummary, BackfillWriter};
pub use clickhouse_schema::{
    ClickHouseSchemaConfig, clickhouse_event_table_ddl, clickhouse_schema_ddl,
};
pub use wal::StorageWal;

pub type FastMap<K, V> = HashMap<K, V, RandomState>;
pub type FastSet<T> = HashSet<T, RandomState>;

#[derive(Clone, Debug)]
pub struct StorageConfig {
    pub event_capacity: usize,
    pub recent_tx_capacity: usize,
    pub table_capacity: usize,
    pub write_latency_capacity: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            event_capacity: 250_000,
            recent_tx_capacity: 25_000,
            table_capacity: 250_000,
            write_latency_capacity: 16_384,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxSeenRecord {
    pub hash: TxHash,
    pub peer: PeerId,
    pub first_seen_unix_ms: i64,
    pub first_seen_mono_ns: u64,
    pub seen_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxFullRecord {
    pub hash: TxHash,
    pub tx_type: u8,
    pub sender: Address,
    pub nonce: u64,
    pub to: Option<Address>,
    pub chain_id: Option<u64>,
    pub value_wei: Option<u128>,
    pub gas_limit: Option<u64>,
    pub gas_price_wei: Option<u128>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub max_fee_per_blob_gas_wei: Option<u128>,
    pub calldata_len: Option<u32>,
    pub raw_tx: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxFeaturesRecord {
    pub hash: TxHash,
    #[serde(default)]
    pub chain_id: Option<u64>,
    pub protocol: String,
    pub category: String,
    pub mev_score: u16,
    pub urgency_score: u16,
    pub method_selector: Option<[u8; 4]>,
    #[serde(default = "default_feature_engine_version")]
    pub feature_engine_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpportunityRecord {
    pub tx_hash: TxHash,
    #[serde(default)]
    pub chain_id: Option<u64>,
    pub strategy: String,
    pub score: u32,
    pub protocol: String,
    pub category: String,
    pub feature_engine_version: String,
    pub scorer_version: String,
    pub strategy_version: String,
    pub reasons: Vec<String>,
    pub detected_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxLifecycleRecord {
    pub hash: TxHash,
    pub status: String,
    pub reason: Option<String>,
    pub updated_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerStatsRecord {
    pub peer: PeerId,
    pub throughput_tps: u32,
    pub drop_rate_bps: u16,
    pub rtt_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecentTransactionRecord {
    pub hash: TxHash,
    pub sender: Address,
    pub nonce: u64,
    pub tx_type: u8,
    pub seen_unix_ms: i64,
    pub source_id: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketStatsSnapshot {
    pub total_signal_volume: u64,
    pub total_tx_count: u64,
    pub low_risk_count: u64,
    pub medium_risk_count: u64,
    pub high_risk_count: u64,
}

fn default_feature_engine_version() -> String {
    feature_engine::version().to_owned()
}

pub trait EventStore {
    fn append_event(&mut self, event: EventEnvelope);
    fn list_events(&self) -> Vec<EventEnvelope>;
    fn scan_events(&self, from_seq_id: u64, limit: usize) -> Vec<EventEnvelope>;
    fn latest_seq_id(&self) -> Option<u64>;
}

pub fn scan_events_cursor_start(events: &[EventEnvelope], from_seq_id: u64) -> usize {
    events.partition_point(|event| event.seq_id <= from_seq_id)
}

#[derive(Clone, Debug)]
pub struct InMemoryStorage {
    config: StorageConfig,
    events: VecDeque<EventEnvelope>,
    event_index: Vec<EventEnvelope>,
    tx_seen: VecDeque<TxSeenRecord>,
    tx_full: VecDeque<TxFullRecord>,
    tx_features: VecDeque<TxFeaturesRecord>,
    opportunities: VecDeque<OpportunityRecord>,
    tx_lifecycle: VecDeque<TxLifecycleRecord>,
    peer_stats: VecDeque<PeerStatsRecord>,
    write_latency_ns: VecDeque<u64>,
    recent_tx_order: VecDeque<TxHash>,
    recent_tx_counts: FastMap<TxHash, usize>,
    recent_tx_lookup: FastMap<TxHash, RecentTransactionRecord>,
    market_stats: MarketStatsSnapshot,
    read_model_revision: u64,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::with_config(StorageConfig::default())
    }
}

impl InMemoryStorage {
    pub fn with_config(config: StorageConfig) -> Self {
        let config = StorageConfig {
            event_capacity: config.event_capacity.max(1),
            recent_tx_capacity: config.recent_tx_capacity.max(1),
            table_capacity: config.table_capacity.max(1),
            write_latency_capacity: config.write_latency_capacity.max(1),
        };

        Self {
            config,
            events: VecDeque::new(),
            event_index: Vec::new(),
            tx_seen: VecDeque::new(),
            tx_full: VecDeque::new(),
            tx_features: VecDeque::new(),
            opportunities: VecDeque::new(),
            tx_lifecycle: VecDeque::new(),
            peer_stats: VecDeque::new(),
            write_latency_ns: VecDeque::new(),
            recent_tx_order: VecDeque::new(),
            recent_tx_counts: FastMap::default(),
            recent_tx_lookup: FastMap::default(),
            market_stats: MarketStatsSnapshot::default(),
            read_model_revision: 0,
        }
    }

    pub fn upsert_tx_seen(&mut self, record: TxSeenRecord) {
        let start = Instant::now();
        push_bounded(&mut self.tx_seen, record, self.config.table_capacity);
        self.market_stats.total_tx_count = self.market_stats.total_tx_count.saturating_add(1);
        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    pub fn upsert_tx_full(&mut self, record: TxFullRecord) {
        let start = Instant::now();
        push_bounded(&mut self.tx_full, record, self.config.table_capacity);
        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    pub fn upsert_tx_features(&mut self, record: TxFeaturesRecord) {
        let start = Instant::now();
        let mev_score = record.mev_score;
        push_bounded(&mut self.tx_features, record, self.config.table_capacity);
        self.market_stats.total_signal_volume =
            self.market_stats.total_signal_volume.saturating_add(1);
        if mev_score >= 80 {
            self.market_stats.high_risk_count = self.market_stats.high_risk_count.saturating_add(1);
        } else if mev_score >= 45 {
            self.market_stats.medium_risk_count =
                self.market_stats.medium_risk_count.saturating_add(1);
        } else {
            self.market_stats.low_risk_count = self.market_stats.low_risk_count.saturating_add(1);
        }
        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    pub fn upsert_opportunity(&mut self, record: OpportunityRecord) {
        let start = Instant::now();
        push_bounded(&mut self.opportunities, record, self.config.table_capacity);
        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    pub fn upsert_tx_lifecycle(&mut self, record: TxLifecycleRecord) {
        let start = Instant::now();
        push_bounded(&mut self.tx_lifecycle, record, self.config.table_capacity);
        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    pub fn upsert_peer_stats(&mut self, record: PeerStatsRecord) {
        let start = Instant::now();
        push_bounded(&mut self.peer_stats, record, self.config.table_capacity);
        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    pub fn tx_seen(&self) -> &VecDeque<TxSeenRecord> {
        &self.tx_seen
    }

    pub fn tx_full(&self) -> &VecDeque<TxFullRecord> {
        &self.tx_full
    }

    pub fn tx_features(&self) -> &VecDeque<TxFeaturesRecord> {
        &self.tx_features
    }

    pub fn opportunities(&self) -> Vec<OpportunityRecord> {
        let mut out = self.opportunities.iter().cloned().collect::<Vec<_>>();
        out.sort_unstable_by(|a, b| {
            b.detected_unix_ms
                .cmp(&a.detected_unix_ms)
                .then_with(|| a.tx_hash.cmp(&b.tx_hash))
                .then_with(|| a.strategy.cmp(&b.strategy))
        });
        out
    }

    pub fn tx_lifecycle(&self) -> &VecDeque<TxLifecycleRecord> {
        &self.tx_lifecycle
    }

    pub fn peer_stats(&self) -> &VecDeque<PeerStatsRecord> {
        &self.peer_stats
    }

    pub fn recent_transactions(&self, limit: usize) -> Vec<RecentTransactionRecord> {
        let target = limit.max(1);
        let mut out = Vec::with_capacity(target);
        let mut emitted_hashes: FastSet<TxHash> = FastSet::default();

        for hash in self.recent_tx_order.iter().rev() {
            if !emitted_hashes.insert(*hash) {
                continue;
            }
            if let Some(record) = self.recent_tx_lookup.get(hash) {
                out.push(record.clone());
                if out.len() >= target {
                    break;
                }
            }
        }

        out.sort_unstable_by(|a, b| {
            b.seen_unix_ms
                .cmp(&a.seen_unix_ms)
                .then_with(|| a.hash.cmp(&b.hash))
        });
        out
    }

    pub fn avg_write_latency_ns(&self) -> Option<u64> {
        if self.write_latency_ns.is_empty() {
            return None;
        }
        Some(
            self.write_latency_ns
                .iter()
                .sum::<u64>()
                .saturating_div(self.write_latency_ns.len() as u64),
        )
    }

    pub fn market_stats_snapshot(&self) -> MarketStatsSnapshot {
        self.market_stats
    }

    pub fn read_model_revision(&self) -> u64 {
        self.read_model_revision
    }

    fn track_recent_transaction(&mut self, record: RecentTransactionRecord) {
        let hash = record.hash;
        self.recent_tx_order.push_back(hash);
        *self.recent_tx_counts.entry(hash).or_insert(0) += 1;
        self.recent_tx_lookup.insert(hash, record);

        while self.recent_tx_order.len() > self.config.recent_tx_capacity {
            if let Some(old_hash) = self.recent_tx_order.pop_front()
                && let Some(count) = self.recent_tx_counts.get_mut(&old_hash)
            {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.recent_tx_counts.remove(&old_hash);
                    self.recent_tx_lookup.remove(&old_hash);
                }
            }
        }
    }

    fn record_write_latency(&mut self, latency_ns: u64) {
        push_bounded(
            &mut self.write_latency_ns,
            latency_ns,
            self.config.write_latency_capacity,
        );
    }

    fn bump_read_model_revision(&mut self) {
        self.read_model_revision = self.read_model_revision.saturating_add(1);
    }
}

impl EventStore for InMemoryStorage {
    fn append_event(&mut self, event: EventEnvelope) {
        let start = Instant::now();

        if let EventPayload::TxDecoded(decoded) = &event.payload {
            self.track_recent_transaction(RecentTransactionRecord {
                hash: decoded.hash,
                sender: decoded.sender,
                nonce: decoded.nonce,
                tx_type: decoded.tx_type,
                seen_unix_ms: event.ingest_ts_unix_ms,
                source_id: event.source_id.0.clone(),
            });
        }

        let lifecycle_update = match &event.payload {
            EventPayload::TxDecoded(decoded) => Some(TxLifecycleRecord {
                hash: decoded.hash,
                status: "pending".to_owned(),
                reason: None,
                updated_unix_ms: event.ingest_ts_unix_ms,
            }),
            EventPayload::TxDropped(dropped) => Some(TxLifecycleRecord {
                hash: dropped.hash,
                status: "dropped".to_owned(),
                reason: Some(dropped.reason.clone()),
                updated_unix_ms: event.ingest_ts_unix_ms,
            }),
            EventPayload::TxConfirmedProvisional(confirmed) => Some(TxLifecycleRecord {
                hash: confirmed.hash,
                status: "confirmed_provisional".to_owned(),
                reason: Some(format!(
                    "block={}:{}",
                    confirmed.block_number,
                    format_hash(&confirmed.block_hash)
                )),
                updated_unix_ms: event.ingest_ts_unix_ms,
            }),
            EventPayload::TxConfirmedFinal(confirmed) => Some(TxLifecycleRecord {
                hash: confirmed.hash,
                status: "confirmed_final".to_owned(),
                reason: Some(format!(
                    "block={}:{}",
                    confirmed.block_number,
                    format_hash(&confirmed.block_hash)
                )),
                updated_unix_ms: event.ingest_ts_unix_ms,
            }),
            EventPayload::TxReplaced(replaced) => Some(TxLifecycleRecord {
                hash: replaced.hash,
                status: "replaced".to_owned(),
                reason: Some(format_hash(&replaced.replaced_by)),
                updated_unix_ms: event.ingest_ts_unix_ms,
            }),
            EventPayload::TxReorged(reorged) => Some(TxLifecycleRecord {
                hash: reorged.hash,
                status: "pending".to_owned(),
                reason: Some("reorg_reopened".to_owned()),
                updated_unix_ms: event.ingest_ts_unix_ms,
            }),
            EventPayload::TxSeen(_)
            | EventPayload::TxFetched(_)
            | EventPayload::OppDetected(_)
            | EventPayload::SimCompleted(_)
            | EventPayload::BundleSubmitted(_) => None,
        };
        if let Some(record) = lifecycle_update {
            self.upsert_tx_lifecycle(record);
        }

        let insert_at = self
            .event_index
            .partition_point(|existing| cmp_deterministic(existing, &event).is_lt());
        self.event_index.insert(insert_at, event.clone());
        self.events.push_back(event);
        while self.events.len() > self.config.event_capacity {
            if let Some(old_event) = self.events.pop_front() {
                remove_event_from_sorted_index(&mut self.event_index, &old_event);
            }
        }

        self.record_write_latency(start.elapsed().as_nanos() as u64);
        self.bump_read_model_revision();
    }

    fn list_events(&self) -> Vec<EventEnvelope> {
        self.event_index.clone()
    }

    fn scan_events(&self, from_seq_id: u64, limit: usize) -> Vec<EventEnvelope> {
        let limit = limit.max(1);
        let start = scan_events_cursor_start(&self.event_index, from_seq_id);
        self.event_index[start..]
            .iter()
            .take(limit)
            .cloned()
            .collect()
    }

    fn latest_seq_id(&self) -> Option<u64> {
        self.event_index.last().map(|event| event.seq_id)
    }
}

#[derive(Clone, Debug)]
pub enum StorageWriteOp {
    AppendEvent(EventEnvelope),
    UpsertTxSeen(TxSeenRecord),
    UpsertTxFull(TxFullRecord),
    UpsertTxFeatures(TxFeaturesRecord),
    UpsertOpportunity(OpportunityRecord),
    UpsertTxLifecycle(TxLifecycleRecord),
    UpsertPeerStats(PeerStatsRecord),
}

#[derive(Clone, Debug)]
pub struct StorageWriterConfig {
    pub queue_capacity: usize,
    pub flush_batch_size: usize,
    pub flush_interval_ms: u64,
    pub wal_path: Option<PathBuf>,
}

impl Default for StorageWriterConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 8_192,
            flush_batch_size: 512,
            flush_interval_ms: 500,
            wal_path: None,
        }
    }
}

impl StorageWriterConfig {
    pub fn high_throughput_defaults() -> Self {
        Self {
            queue_capacity: 32_768,
            flush_batch_size: 2_048,
            flush_interval_ms: 100,
            wal_path: None,
        }
    }
}

#[derive(Clone)]
pub struct StorageWriteHandle {
    tx: mpsc::Sender<StorageWriteOp>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageTryEnqueueError {
    QueueFull,
    QueueClosed,
}

impl StorageWriteHandle {
    pub fn from_sender(tx: mpsc::Sender<StorageWriteOp>) -> Self {
        Self { tx }
    }

    pub async fn enqueue(&self, op: StorageWriteOp) -> Result<()> {
        self.tx
            .send(op)
            .await
            .map_err(|_| anyhow!("storage writer task is not running"))
    }

    pub fn try_enqueue(
        &self,
        op: StorageWriteOp,
    ) -> std::result::Result<(), StorageTryEnqueueError> {
        self.tx.try_send(op).map_err(|err| match err {
            TrySendError::Full(_) => StorageTryEnqueueError::QueueFull,
            TrySendError::Closed(_) => StorageTryEnqueueError::QueueClosed,
        })
    }
}

#[async_trait]
pub trait ClickHouseBatchSink: Send + Sync {
    async fn flush_event_batch(&self, events: Vec<EventEnvelope>) -> Result<()>;
}

#[derive(Clone, Debug, Default)]
pub struct NoopClickHouseSink;

#[async_trait]
impl ClickHouseBatchSink for NoopClickHouseSink {
    async fn flush_event_batch(&self, _events: Vec<EventEnvelope>) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct ClickHouseHttpSink {
    client: reqwest::Client,
    insert_url: String,
}

impl ClickHouseHttpSink {
    pub fn new(insert_url: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("build clickhouse http client")?;
        Ok(Self {
            client,
            insert_url: insert_url.into(),
        })
    }

    pub fn from_env() -> Result<Option<Self>> {
        let url = match std::env::var("CLICKHOUSE_EVENTS_INSERT_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => return Ok(None),
        };
        Self::new(url).map(Some)
    }
}

#[async_trait]
impl ClickHouseBatchSink for ClickHouseHttpSink {
    async fn flush_event_batch(&self, events: Vec<EventEnvelope>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let mut body = String::new();
        for event in events {
            body.push_str(&serde_json::to_string(&event).context("serialize event")?);
            body.push('\n');
        }

        self.client
            .post(&self.insert_url)
            .header("content-type", "application/x-ndjson")
            .body(body)
            .send()
            .await
            .with_context(|| format!("POST {}", self.insert_url))?
            .error_for_status()
            .context("clickhouse insert returned error status")?;

        Ok(())
    }
}

pub fn spawn_single_writer(
    storage: Arc<RwLock<InMemoryStorage>>,
    sink: Arc<dyn ClickHouseBatchSink>,
    config: StorageWriterConfig,
) -> StorageWriteHandle {
    let config = StorageWriterConfig {
        queue_capacity: config.queue_capacity.max(1),
        flush_batch_size: config.flush_batch_size.max(1),
        flush_interval_ms: config.flush_interval_ms.max(1),
        wal_path: config.wal_path,
    };
    let wal = config
        .wal_path
        .and_then(|path| match StorageWal::new(path) {
            Ok(wal) => Some(wal),
            Err(err) => {
                tracing::warn!(error = %err, "failed to initialize storage WAL");
                None
            }
        });

    if let Some(wal) = wal.as_ref() {
        match wal.recover_events() {
            Ok(events) => {
                if !events.is_empty()
                    && let Ok(mut guard) = storage.write()
                {
                    for event in events {
                        guard.append_event(event);
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to recover events from storage WAL");
            }
        }
    }

    let mut sequencer = GlobalSequencer::from_latest_seq_id(
        storage.read().ok().and_then(|guard| guard.latest_seq_id()),
    );

    let (tx, mut rx) = mpsc::channel::<StorageWriteOp>(config.queue_capacity);

    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(config.flush_batch_size);
        let mut ticker = tokio::time::interval(Duration::from_millis(config.flush_interval_ms));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                maybe_op = rx.recv() => {
                    let op = match maybe_op {
                        Some(op) => op,
                        None => break,
                    };

                    match storage.write() {
                        Ok(mut guard) => apply_write_op(
                            &mut guard,
                            op,
                            &mut batch,
                            wal.as_ref(),
                            &mut sequencer,
                        ),
                        Err(_) => {
                            tracing::error!("storage lock poisoned, stopping single-writer task");
                            break;
                        }
                    }

                    if batch.len() >= config.flush_batch_size {
                        flush_batch(&sink, &mut batch, wal.as_ref()).await;
                    }
                }
                _ = ticker.tick() => {
                    if !batch.is_empty() {
                        flush_batch(&sink, &mut batch, wal.as_ref()).await;
                    }
                }
            }
        }

        if !batch.is_empty() {
            flush_batch(&sink, &mut batch, wal.as_ref()).await;
        }
    });

    StorageWriteHandle { tx }
}

fn apply_write_op(
    storage: &mut InMemoryStorage,
    op: StorageWriteOp,
    batch: &mut Vec<EventEnvelope>,
    wal: Option<&StorageWal>,
    sequencer: &mut GlobalSequencer,
) {
    match op {
        StorageWriteOp::AppendEvent(event) => {
            let event = sequencer.assign(event);
            if let Some(wal) = wal
                && let Err(err) = wal.append_event(&event)
            {
                tracing::warn!(error = %err, "failed to append event to storage WAL");
            }
            storage.append_event(event.clone());
            batch.push(event);
        }
        StorageWriteOp::UpsertTxSeen(record) => storage.upsert_tx_seen(record),
        StorageWriteOp::UpsertTxFull(record) => storage.upsert_tx_full(record),
        StorageWriteOp::UpsertTxFeatures(record) => storage.upsert_tx_features(record),
        StorageWriteOp::UpsertOpportunity(record) => storage.upsert_opportunity(record),
        StorageWriteOp::UpsertTxLifecycle(record) => storage.upsert_tx_lifecycle(record),
        StorageWriteOp::UpsertPeerStats(record) => storage.upsert_peer_stats(record),
    }
}

async fn flush_batch(
    sink: &Arc<dyn ClickHouseBatchSink>,
    batch: &mut Vec<EventEnvelope>,
    wal: Option<&StorageWal>,
) {
    if batch.is_empty() {
        return;
    }
    let pending = std::mem::take(batch);
    if let Err(err) = sink.flush_event_batch(pending).await {
        tracing::warn!(error = %err, "clickhouse batch flush failed");
    } else if let Some(wal) = wal
        && let Err(err) = wal.clear()
    {
        tracing::warn!(error = %err, "failed to clear storage WAL after flush");
    }
}

fn push_bounded<T>(deque: &mut VecDeque<T>, value: T, capacity: usize) {
    deque.push_back(value);
    while deque.len() > capacity {
        let _ = deque.pop_front();
    }
}

fn remove_event_from_sorted_index(events: &mut Vec<EventEnvelope>, target: &EventEnvelope) {
    let mut index = match events.binary_search_by(|existing| cmp_deterministic(existing, target)) {
        Ok(index) => index,
        Err(_) => return,
    };

    while index > 0 && cmp_deterministic(&events[index - 1], target).is_eq() {
        index -= 1;
    }

    while index < events.len() && cmp_deterministic(&events[index], target).is_eq() {
        if events[index] == *target {
            events.remove(index);
            return;
        }
        index += 1;
    }
}

pub fn export_replay_frames_json(frames: &[ReplayFrame]) -> Result<String> {
    serde_json::to_string_pretty(frames).map_err(|err| anyhow!(err))
}

pub fn export_replay_frames_csv(frames: &[ReplayFrame]) -> String {
    let mut out = String::from("seq_hi,timestamp_unix_ms,pending_hashes\n");
    for frame in frames {
        let hashes = frame
            .pending
            .iter()
            .map(format_hash)
            .collect::<Vec<_>>()
            .join(";");
        out.push_str(&format!(
            "{},{},{}\n",
            frame.seq_hi, frame.timestamp_unix_ms, hashes
        ));
    }
    out
}

pub trait ParquetExporter {
    fn export_replay_frames_parquet(&self, _frames: &[ReplayFrame], _path: &str) -> Result<()> {
        Err(anyhow!(
            "parquet export is not wired in this crate yet; use adapter implementation"
        ))
    }
}

#[derive(Clone, Debug, Default)]
pub struct UnsupportedParquetExporter;

impl ParquetExporter for UnsupportedParquetExporter {}

#[derive(Clone, Debug, Default)]
pub struct ArrowParquetExporter;

impl ParquetExporter for ArrowParquetExporter {
    fn export_replay_frames_parquet(&self, frames: &[ReplayFrame], path: &str) -> Result<()> {
        let payload = serde_json::to_vec_pretty(frames).context("serialize replay frames")?;
        fs::write(path, payload).with_context(|| format!("write replay export file {path}"))?;
        Ok(())
    }
}

fn format_hash(hash: &TxHash) -> String {
    let mut out = String::from("0x");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::SourceId;
    use event_log::{EventPayload, TxConfirmed, TxDecoded, TxReorged, TxSeen};

    fn hash(v: u8) -> TxHash {
        [v; 32]
    }

    fn decoded_event(seq: u64, v: u8) -> EventEnvelope {
        EventEnvelope {
            seq_id: seq,
            ingest_ts_unix_ms: 1_700_000_000_000 + seq as i64,
            ingest_ts_mono_ns: seq * 10,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxDecoded(TxDecoded {
                hash: hash(v),
                tx_type: 2,
                sender: [9; 20],
                nonce: seq,
                chain_id: Some(1),
                to: None,
                value_wei: None,
                gas_limit: None,
                gas_price_wei: None,
                max_fee_per_gas_wei: None,
                max_priority_fee_per_gas_wei: None,
                max_fee_per_blob_gas_wei: None,
                calldata_len: None,
            }),
        }
    }

    fn seen_event(seq: u64, source: &str, hash_seed: u8) -> EventEnvelope {
        EventEnvelope {
            seq_id: seq,
            ingest_ts_unix_ms: 1_700_000_000_000 + seq as i64,
            ingest_ts_mono_ns: seq * 10,
            source_id: SourceId::new(source),
            payload: EventPayload::TxSeen(TxSeen {
                hash: hash(hash_seed),
                peer_id: format!("peer-{hash_seed}"),
                seen_at_unix_ms: 1_700_000_000_000 + seq as i64,
                seen_at_mono_ns: seq * 10,
            }),
        }
    }

    fn decoded_event_with_ts(seq: u64, ts_unix_ms: i64, hash_seed: u8) -> EventEnvelope {
        EventEnvelope {
            seq_id: seq,
            ingest_ts_unix_ms: ts_unix_ms,
            ingest_ts_mono_ns: seq * 10,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxDecoded(TxDecoded {
                hash: hash(hash_seed),
                tx_type: 2,
                sender: [9; 20],
                nonce: seq,
                chain_id: Some(1),
                to: None,
                value_wei: None,
                gas_limit: None,
                gas_price_wei: None,
                max_fee_per_gas_wei: None,
                max_priority_fee_per_gas_wei: None,
                max_fee_per_blob_gas_wei: None,
                calldata_len: None,
            }),
        }
    }

    fn opportunity_record(seq: u8) -> OpportunityRecord {
        OpportunityRecord {
            tx_hash: hash(seq),
            chain_id: Some(1),
            strategy: "SandwichCandidate".to_owned(),
            score: 10_000 + seq as u32,
            protocol: "uniswap-v2".to_owned(),
            category: "swap".to_owned(),
            feature_engine_version: "feature-engine.v1".to_owned(),
            scorer_version: "scorer.v1".to_owned(),
            strategy_version: "strategy.sandwich.v1".to_owned(),
            reasons: vec!["mev_score".to_owned()],
            detected_unix_ms: 1_700_000_000_000 + seq as i64,
        }
    }

    #[test]
    fn in_memory_storage_inserts_and_queries_records() {
        let mut store = InMemoryStorage::default();

        store.upsert_tx_seen(TxSeenRecord {
            hash: hash(1),
            peer: "peer-a".to_owned(),
            first_seen_unix_ms: 1_700_000_000_000,
            first_seen_mono_ns: 10,
            seen_count: 1,
        });
        store.upsert_tx_full(TxFullRecord {
            hash: hash(1),
            tx_type: 2,
            sender: [9; 20],
            nonce: 1,
            to: None,
            chain_id: Some(1),
            value_wei: None,
            gas_limit: None,
            gas_price_wei: None,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(3),
            raw_tx: vec![1, 2, 3],
        });
        store.upsert_tx_features(TxFeaturesRecord {
            hash: hash(1),
            chain_id: Some(1),
            protocol: "uniswap-v2".to_owned(),
            category: "swap".to_owned(),
            mev_score: 80,
            urgency_score: 40,
            method_selector: Some([0x38, 0xed, 0x17, 0x39]),
            feature_engine_version: "feature-engine.v1".to_owned(),
        });
        store.upsert_tx_lifecycle(TxLifecycleRecord {
            hash: hash(1),
            status: "pending".to_owned(),
            reason: None,
            updated_unix_ms: 1_700_000_000_001,
        });
        store.upsert_peer_stats(PeerStatsRecord {
            peer: "peer-a".to_owned(),
            throughput_tps: 100,
            drop_rate_bps: 12,
            rtt_ms: 5,
        });

        assert_eq!(store.tx_seen().len(), 1);
        assert_eq!(store.tx_full().len(), 1);
        assert_eq!(store.tx_features().len(), 1);
        assert_eq!(
            store.tx_features()[0].feature_engine_version,
            "feature-engine.v1"
        );
        assert_eq!(store.tx_lifecycle().len(), 1);
        assert_eq!(store.peer_stats().len(), 1);
        assert!(store.avg_write_latency_ns().is_some());
    }

    #[test]
    fn event_ring_buffer_applies_capacity() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 2,
            recent_tx_capacity: 10,
            ..StorageConfig::default()
        });

        store.append_event(decoded_event(1, 1));
        store.append_event(decoded_event(2, 2));
        store.append_event(decoded_event(3, 3));

        let events = store.list_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq_id, 2);
        assert_eq!(events[1].seq_id, 3);
    }

    #[test]
    fn recent_transactions_is_bounded_and_latest_first() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 100,
            recent_tx_capacity: 3,
            ..StorageConfig::default()
        });

        store.append_event(decoded_event(1, 1));
        store.append_event(decoded_event(2, 2));
        store.append_event(decoded_event(3, 3));
        store.append_event(decoded_event(4, 4));

        let recent = store.recent_transactions(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].hash, hash(4));
        assert_eq!(recent[2].hash, hash(2));
    }

    #[tokio::test]
    async fn single_writer_applies_ops_and_batches_flushes() {
        #[derive(Clone)]
        struct RecordingSink {
            flush_sizes: Arc<RwLock<Vec<usize>>>,
        }

        #[async_trait]
        impl ClickHouseBatchSink for RecordingSink {
            async fn flush_event_batch(&self, events: Vec<EventEnvelope>) -> Result<()> {
                self.flush_sizes.write().unwrap().push(events.len());
                Ok(())
            }
        }

        let storage = Arc::new(RwLock::new(InMemoryStorage::with_config(StorageConfig {
            event_capacity: 4,
            recent_tx_capacity: 4,
            ..StorageConfig::default()
        })));
        let flush_sizes = Arc::new(RwLock::new(Vec::new()));
        let sink = Arc::new(RecordingSink {
            flush_sizes: flush_sizes.clone(),
        });

        let handle = spawn_single_writer(
            storage.clone(),
            sink,
            StorageWriterConfig {
                queue_capacity: 32,
                flush_batch_size: 2,
                flush_interval_ms: 5,
                wal_path: None,
            },
        );

        for seq in 1..=5 {
            handle
                .enqueue(StorageWriteOp::AppendEvent(decoded_event(seq, seq as u8)))
                .await
                .expect("enqueue append event");
        }

        tokio::time::sleep(Duration::from_millis(30)).await;

        let events = storage.read().unwrap().list_events();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].seq_id, 2);
        assert_eq!(events[3].seq_id, 5);

        let flushed = flush_sizes.read().unwrap().clone();
        assert!(!flushed.is_empty());
    }

    #[tokio::test]
    async fn single_writer_assigns_global_sequence_for_colliding_lane_seq_ids() {
        let storage = Arc::new(RwLock::new(InMemoryStorage::with_config(StorageConfig {
            event_capacity: 8,
            recent_tx_capacity: 8,
            ..StorageConfig::default()
        })));
        let sink = Arc::new(NoopClickHouseSink);

        let handle = spawn_single_writer(
            storage.clone(),
            sink,
            StorageWriterConfig {
                queue_capacity: 32,
                flush_batch_size: 16,
                flush_interval_ms: 20,
                wal_path: None,
            },
        );

        // Local lane sequence IDs intentionally collide (rpc/p2p both emit seq 1, 2).
        handle
            .enqueue(StorageWriteOp::AppendEvent(seen_event(1, "rpc-mainnet", 1)))
            .await
            .expect("enqueue rpc seq1");
        handle
            .enqueue(StorageWriteOp::AppendEvent(seen_event(1, "p2p-mainnet", 2)))
            .await
            .expect("enqueue p2p seq1");
        handle
            .enqueue(StorageWriteOp::AppendEvent(seen_event(2, "rpc-mainnet", 3)))
            .await
            .expect("enqueue rpc seq2");

        tokio::time::sleep(Duration::from_millis(40)).await;

        let events = storage.read().expect("storage lock").list_events();
        let seq_ids = events.iter().map(|event| event.seq_id).collect::<Vec<_>>();
        assert_eq!(seq_ids, vec![1, 2, 3]);
    }

    #[test]
    fn event_store_appends_events() {
        let mut store = InMemoryStorage::default();
        store.append_event(EventEnvelope {
            seq_id: 1,
            ingest_ts_unix_ms: 1_700_000_000_000,
            ingest_ts_mono_ns: 10,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxSeen(TxSeen {
                hash: hash(2),
                peer_id: "peer-a".to_owned(),
                seen_at_unix_ms: 1_700_000_000_000,
                seen_at_mono_ns: 10,
            }),
        });
        assert_eq!(store.list_events().len(), 1);
    }

    #[test]
    fn list_events_returns_deterministic_order() {
        let mut store = InMemoryStorage::default();
        // Deliberately append out of canonical order.
        store.append_event(seen_event(10, "peer-b", 2));
        store.append_event(seen_event(9, "peer-z", 3));
        store.append_event(seen_event(10, "peer-a", 4));

        let events = store.list_events();
        let keys = events
            .into_iter()
            .map(|event| {
                let hash = event.payload.primary_hash();
                (event.seq_id, event.source_id.0, hash)
            })
            .collect::<Vec<_>>();

        assert_eq!(keys[0].0, 9);
        assert_eq!(keys[1].0, 10);
        assert_eq!(keys[2].0, 10);
        assert!(keys[1].1 < keys[2].1);
    }

    #[test]
    fn scan_events_returns_sorted_window_after_seq() {
        let mut store = InMemoryStorage::default();
        store.append_event(seen_event(5, "peer-b", 5));
        store.append_event(seen_event(3, "peer-a", 3));
        store.append_event(seen_event(4, "peer-c", 4));

        let scan = store.scan_events(3, 2);
        let seqs = scan
            .into_iter()
            .map(|event| event.seq_id)
            .collect::<Vec<_>>();
        assert_eq!(seqs, vec![4, 5]);
    }

    #[test]
    fn recent_transactions_are_sorted_by_seen_time_desc_then_hash() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 100,
            recent_tx_capacity: 10,
            ..StorageConfig::default()
        });

        store.append_event(decoded_event_with_ts(1, 1_700_000_000_100, 1));
        store.append_event(decoded_event_with_ts(2, 1_700_000_000_300, 2));
        store.append_event(decoded_event_with_ts(3, 1_700_000_000_200, 3));

        let recent = store.recent_transactions(10);
        let order = recent.into_iter().map(|row| row.hash).collect::<Vec<_>>();
        assert_eq!(order, vec![hash(2), hash(3), hash(1)]);
    }

    #[test]
    fn replay_export_outputs_are_deterministic() {
        let frames = vec![
            ReplayFrame {
                seq_hi: 1,
                timestamp_unix_ms: 1_700_000_000_000,
                pending: vec![hash(3)],
            },
            ReplayFrame {
                seq_hi: 2,
                timestamp_unix_ms: 1_700_000_000_100,
                pending: vec![hash(3), hash(4)],
            },
        ];
        let json_a = export_replay_frames_json(&frames).expect("json");
        let json_b = export_replay_frames_json(&frames).expect("json");
        assert_eq!(json_a, json_b);

        let csv = export_replay_frames_csv(&frames);
        assert!(csv.contains("seq_hi,timestamp_unix_ms,pending_hashes"));
        assert!(csv.contains("0x03030303"));
    }

    #[test]
    fn unsupported_parquet_adapter_returns_error() {
        let exporter = UnsupportedParquetExporter;
        let frames = vec![ReplayFrame {
            seq_hi: 1,
            timestamp_unix_ms: 1_700_000_000_000,
            pending: vec![hash(9)],
        }];
        let err = exporter
            .export_replay_frames_parquet(&frames, "/tmp/replay.parquet")
            .expect_err("should return adapter error");
        assert!(
            err.to_string()
                .contains("parquet export is not wired in this crate yet")
        );
    }

    #[test]
    fn side_tables_apply_configured_capacity() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 100,
            recent_tx_capacity: 100,
            table_capacity: 2,
            write_latency_capacity: 100,
        });

        for idx in 0..4_u8 {
            let hash = hash(idx + 1);
            store.upsert_tx_seen(TxSeenRecord {
                hash,
                peer: "peer-a".to_owned(),
                first_seen_unix_ms: 1_700_000_000_000 + idx as i64,
                first_seen_mono_ns: idx as u64,
                seen_count: 1,
            });
            store.upsert_tx_full(TxFullRecord {
                hash,
                tx_type: 2,
                sender: [9; 20],
                nonce: idx as u64,
                to: None,
                chain_id: Some(1),
                value_wei: None,
                gas_limit: None,
                gas_price_wei: None,
                max_fee_per_gas_wei: None,
                max_priority_fee_per_gas_wei: None,
                max_fee_per_blob_gas_wei: None,
                calldata_len: Some(1),
                raw_tx: vec![idx],
            });
            store.upsert_tx_features(TxFeaturesRecord {
                hash,
                chain_id: Some(1),
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                mev_score: 80,
                urgency_score: 40,
                method_selector: Some([0x38, 0xed, 0x17, 0x39]),
                feature_engine_version: "feature-engine.v1".to_owned(),
            });
            store.upsert_tx_lifecycle(TxLifecycleRecord {
                hash,
                status: "pending".to_owned(),
                reason: None,
                updated_unix_ms: 1_700_000_000_000 + idx as i64,
            });
            store.upsert_peer_stats(PeerStatsRecord {
                peer: "peer-a".to_owned(),
                throughput_tps: 100,
                drop_rate_bps: 10,
                rtt_ms: 5,
            });
        }

        assert_eq!(store.tx_seen().len(), 2);
        assert_eq!(store.tx_full().len(), 2);
        assert_eq!(store.tx_features().len(), 2);
        assert_eq!(store.tx_lifecycle().len(), 2);
        assert_eq!(store.peer_stats().len(), 2);
        let market_stats = store.market_stats_snapshot();
        assert_eq!(market_stats.total_tx_count, 4);
        assert_eq!(market_stats.total_signal_volume, 4);
        assert_eq!(market_stats.low_risk_count, 0);
        assert_eq!(market_stats.medium_risk_count, 0);
        assert_eq!(market_stats.high_risk_count, 4);
    }

    #[test]
    fn market_stats_counters_are_cumulative_even_when_tables_are_bounded() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 100,
            recent_tx_capacity: 100,
            table_capacity: 1,
            write_latency_capacity: 100,
        });

        for (idx, mev_score) in [10_u16, 45, 80].into_iter().enumerate() {
            let hash = hash((idx + 1) as u8);
            store.upsert_tx_seen(TxSeenRecord {
                hash,
                peer: "peer-a".to_owned(),
                first_seen_unix_ms: 1_700_000_000_000 + idx as i64,
                first_seen_mono_ns: idx as u64,
                seen_count: 1,
            });
            store.upsert_tx_features(TxFeaturesRecord {
                hash,
                chain_id: Some(1),
                protocol: "erc20".to_owned(),
                category: "transfer".to_owned(),
                mev_score,
                urgency_score: 1,
                method_selector: None,
                feature_engine_version: "feature-engine.v1".to_owned(),
            });
        }

        assert_eq!(store.tx_seen().len(), 1);
        assert_eq!(store.tx_features().len(), 1);
        let market_stats = store.market_stats_snapshot();
        assert_eq!(market_stats.total_tx_count, 3);
        assert_eq!(market_stats.total_signal_volume, 3);
        assert_eq!(market_stats.low_risk_count, 1);
        assert_eq!(market_stats.medium_risk_count, 1);
        assert_eq!(market_stats.high_risk_count, 1);
    }

    #[test]
    fn opportunities_table_is_bounded_and_latest_first() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 100,
            recent_tx_capacity: 100,
            table_capacity: 2,
            write_latency_capacity: 100,
        });

        store.upsert_opportunity(opportunity_record(1));
        store.upsert_opportunity(opportunity_record(2));
        store.upsert_opportunity(opportunity_record(3));

        let rows = store.opportunities();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tx_hash, hash(3));
        assert_eq!(rows[1].tx_hash, hash(2));
    }

    #[test]
    fn write_latency_window_is_bounded() {
        let mut store = InMemoryStorage::with_config(StorageConfig {
            event_capacity: 100,
            recent_tx_capacity: 100,
            table_capacity: 100,
            write_latency_capacity: 4,
        });

        for seq in 1..=32 {
            store.append_event(decoded_event(seq, seq as u8));
        }

        assert_eq!(store.write_latency_ns.len(), 4);
        assert!(store.avg_write_latency_ns().is_some());
    }

    #[test]
    fn append_event_derives_lifecycle_rows_including_reorg_reopen() {
        let mut store = InMemoryStorage::default();
        let tx_hash = hash(42);

        store.append_event(EventEnvelope {
            seq_id: 1,
            ingest_ts_unix_ms: 1_700_000_000_001,
            ingest_ts_mono_ns: 1,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxDecoded(TxDecoded {
                hash: tx_hash,
                tx_type: 2,
                sender: [9; 20],
                nonce: 1,
                chain_id: Some(1),
                to: None,
                value_wei: None,
                gas_limit: None,
                gas_price_wei: None,
                max_fee_per_gas_wei: None,
                max_priority_fee_per_gas_wei: None,
                max_fee_per_blob_gas_wei: None,
                calldata_len: None,
            }),
        });
        store.append_event(EventEnvelope {
            seq_id: 2,
            ingest_ts_unix_ms: 1_700_000_000_002,
            ingest_ts_mono_ns: 2,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxConfirmedProvisional(TxConfirmed {
                hash: tx_hash,
                block_number: 1_234_567,
                block_hash: [7; 32],
            }),
        });
        store.append_event(EventEnvelope {
            seq_id: 3,
            ingest_ts_unix_ms: 1_700_000_000_003,
            ingest_ts_mono_ns: 3,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxReorged(TxReorged {
                hash: tx_hash,
                old_block_hash: [7; 32],
                new_block_hash: [8; 32],
            }),
        });

        let lifecycle = store
            .tx_lifecycle()
            .iter()
            .rev()
            .find(|row| row.hash == tx_hash);
        let lifecycle = lifecycle.expect("lifecycle row");
        assert_eq!(lifecycle.status, "pending");
        assert_eq!(lifecycle.reason.as_deref(), Some("reorg_reopened"));
        assert_eq!(lifecycle.updated_unix_ms, 1_700_000_000_003);
    }
}
