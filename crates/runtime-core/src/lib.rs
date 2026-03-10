#![forbid(unsafe_code)]

pub mod live_rpc;

use anyhow::Result;
use builder::{AssemblyEngine, AssemblyMetrics, AssemblySnapshot};
use common::Address;
use scheduler::{SchedulerHandle, SchedulerMetrics, SchedulerSnapshot};
use serde::{Deserialize, Serialize};
use sim_engine::AccountSeed;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use storage::{InMemoryStorage, OpportunityRecord, StorageWriteHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RuntimeIngestMode {
    Rpc,
    P2p,
    Hybrid,
}

impl RuntimeIngestMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::P2p => "p2p",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LiveRpcDropReason {
    StorageQueueFull,
    StorageQueueClosed,
    InvalidPendingHash,
}

impl LiveRpcDropReason {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::StorageQueueFull => "storage_queue_full",
            Self::StorageQueueClosed => "storage_queue_closed",
            Self::InvalidPendingHash => "invalid_pending_hash",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRpcDropMetricsSnapshot {
    pub storage_queue_full: u64,
    pub storage_queue_closed: u64,
    pub invalid_pending_hash: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRpcSearcherMetricsSnapshot {
    pub executable_batches_total: u64,
    pub executable_candidates_total: u64,
    pub executable_bundle_candidates_total: u64,
    pub max_executable_candidates_in_batch: u64,
    pub comparison_batches_total: u64,
    pub executable_top_score_total: u64,
    pub executable_top_score_wins_total: u64,
    pub top_score_ties_total: u64,
    pub overlapping_candidates_total: u64,
    pub executable_only_candidates_total: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRpcSimulationMetricsSnapshot {
    pub enqueued_total: u64,
    pub completed_total: u64,
    pub ok_total: u64,
    pub failed_total: u64,
    pub state_error_total: u64,
    pub timeout_total: u64,
    pub queue_full_drop_total: u64,
    pub stale_drop_total: u64,
    pub queue_depth: u64,
    pub queue_capacity: u64,
    pub inflight_current: u64,
    pub worker_total: u64,
    pub cache_hit_total: u64,
    pub cache_miss_total: u64,
    pub tx_total: u64,
    pub last_latency_ms: u64,
    pub max_latency_ms: u64,
    pub total_latency_ms: u64,
    pub revert_fail_total: u64,
    pub out_of_gas_fail_total: u64,
    pub nonce_mismatch_fail_total: u64,
    pub state_mismatch_fail_total: u64,
    pub unknown_fail_total: u64,
    pub state_rpc_fail_total: u64,
    pub state_timeout_fail_total: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRpcSimulationStatusSnapshot {
    pub id: String,
    pub bundle_id: String,
    pub status: String,
    pub relay_url: String,
    pub attempt_count: usize,
    pub accepted: bool,
    pub fail_category: Option<String>,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRpcChainStatus {
    pub chain_key: String,
    pub chain_id: Option<u64>,
    pub source_id: String,
    pub state: String,
    pub endpoint_index: usize,
    pub endpoint_count: usize,
    pub ws_url: String,
    pub http_url: String,
    pub last_pending_unix_ms: Option<i64>,
    pub silent_for_ms: Option<u64>,
    pub updated_unix_ms: i64,
    pub last_error: Option<String>,
    pub rotation_count: u64,
}

pub struct RuntimeCoreDeps {
    pub storage: Arc<RwLock<InMemoryStorage>>,
    pub writer: StorageWriteHandle,
    pub scheduler: SchedulerHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeCoreConfig {
    pub ingest_mode: RuntimeIngestMode,
    pub rebuild_scheduler_from_rpc: bool,
}

pub struct RuntimeCoreStartArgs {
    pub deps: RuntimeCoreDeps,
    pub config: RuntimeCoreConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CachedAccountSeed {
    seed: AccountSeed,
    block_number: u64,
    cached_at_unix_ms: i64,
}

#[derive(Default)]
struct RemoteStateCache {
    account_seeds: HashMap<(String, Address), CachedAccountSeed>,
}

pub struct RuntimeCore {
    deps: RuntimeCoreDeps,
    config: RuntimeCoreConfig,
    builder_engine: Arc<RwLock<AssemblyEngine>>,
    drop_metrics: Arc<RwLock<LiveRpcDropMetricsSnapshot>>,
    searcher_metrics: Arc<RwLock<LiveRpcSearcherMetricsSnapshot>>,
    simulation_metrics: Arc<RwLock<LiveRpcSimulationMetricsSnapshot>>,
    simulation_status: Arc<RwLock<SimulationStatusStore>>,
    chain_status: Arc<RwLock<BTreeMap<String, LiveRpcChainStatus>>>,
    simulation_cache: Arc<RwLock<RemoteStateCache>>,
    simulation_http_client: reqwest::Client,
    live_rpc_feed_start_count: AtomicU64,
    mono_epoch: Instant,
}

#[derive(Clone)]
pub struct RuntimeCoreHandle {
    inner: Arc<RuntimeCore>,
}

impl RuntimeCore {
    pub fn start(args: RuntimeCoreStartArgs) -> Result<RuntimeCoreHandle> {
        Ok(RuntimeCoreHandle {
            inner: Arc::new(Self {
                deps: args.deps,
                config: args.config,
                builder_engine: Arc::new(RwLock::new(AssemblyEngine::new(
                    builder::AssemblyConfig::default(),
                ))),
                drop_metrics: Arc::new(RwLock::new(LiveRpcDropMetricsSnapshot::default())),
                searcher_metrics: Arc::new(RwLock::new(LiveRpcSearcherMetricsSnapshot::default())),
                simulation_metrics: Arc::new(RwLock::new(
                    LiveRpcSimulationMetricsSnapshot::default(),
                )),
                simulation_status: Arc::new(RwLock::new(SimulationStatusStore::default())),
                chain_status: Arc::new(RwLock::new(BTreeMap::new())),
                simulation_cache: Arc::new(RwLock::new(RemoteStateCache::default())),
                simulation_http_client: reqwest::Client::new(),
                live_rpc_feed_start_count: AtomicU64::new(0),
                mono_epoch: Instant::now(),
            }),
        })
    }
}

impl RuntimeCoreHandle {
    pub fn config(&self) -> RuntimeCoreConfig {
        self.inner.config
    }

    pub fn storage(&self) -> &Arc<RwLock<InMemoryStorage>> {
        &self.inner.deps.storage
    }

    pub fn writer(&self) -> &StorageWriteHandle {
        &self.inner.deps.writer
    }

    pub fn scheduler(&self) -> &SchedulerHandle {
        &self.inner.deps.scheduler
    }

    pub fn scheduler_snapshot(&self) -> SchedulerSnapshot {
        self.inner.deps.scheduler.snapshot()
    }

    pub fn scheduler_metrics(&self) -> SchedulerMetrics {
        self.inner.deps.scheduler.metrics()
    }

    pub fn builder_snapshot(&self) -> AssemblySnapshot {
        self.inner
            .builder_engine
            .read()
            .map(|guard| guard.snapshot())
            .unwrap_or_default()
    }

    pub fn builder_metrics(&self) -> AssemblyMetrics {
        self.inner
            .builder_engine
            .read()
            .map(|guard| guard.metrics())
            .unwrap_or_default()
    }

    pub fn drop_metrics(&self) -> LiveRpcDropMetricsSnapshot {
        self.inner
            .drop_metrics
            .read()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn searcher_metrics(&self) -> LiveRpcSearcherMetricsSnapshot {
        self.inner
            .searcher_metrics
            .read()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn simulation_metrics(&self) -> LiveRpcSimulationMetricsSnapshot {
        self.inner
            .simulation_metrics
            .read()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn simulation_status(&self, id: &str) -> Option<LiveRpcSimulationStatusSnapshot> {
        let store = self.inner.simulation_status.read().ok()?;
        if id == "latest" {
            return store.latest.clone();
        }
        store.by_id.get(id).cloned()
    }

    pub fn simulation_http_client(&self) -> &reqwest::Client {
        &self.inner.simulation_http_client
    }

    pub fn mono_ns(&self) -> u64 {
        self.inner
            .mono_epoch
            .elapsed()
            .as_nanos()
            .min(u64::MAX as u128) as u64
    }

    pub fn chain_status(&self) -> Vec<LiveRpcChainStatus> {
        let now_unix_ms = current_unix_ms();
        self.inner
            .chain_status
            .read()
            .map(|rows| {
                rows.values()
                    .cloned()
                    .map(|mut status| {
                        status.silent_for_ms = status.last_pending_unix_ms.and_then(|last_seen| {
                            let elapsed_ms = now_unix_ms.saturating_sub(last_seen);
                            u64::try_from(elapsed_ms).ok()
                        });
                        status
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn live_rpc_feed_start_count(&self) -> u64 {
        self.inner.live_rpc_feed_start_count.load(Ordering::Relaxed)
    }

    pub fn record_live_rpc_feed_start(&self) {
        self.inner
            .live_rpc_feed_start_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn reset_drop_metrics(&self) {
        if let Ok(mut guard) = self.inner.drop_metrics.write() {
            *guard = LiveRpcDropMetricsSnapshot::default();
        }
    }

    pub fn observe_drop_reason(&self, reason: LiveRpcDropReason) {
        if let Ok(mut guard) = self.inner.drop_metrics.write() {
            match reason {
                LiveRpcDropReason::StorageQueueFull => {
                    guard.storage_queue_full = guard.storage_queue_full.saturating_add(1);
                }
                LiveRpcDropReason::StorageQueueClosed => {
                    guard.storage_queue_closed = guard.storage_queue_closed.saturating_add(1);
                }
                LiveRpcDropReason::InvalidPendingHash => {
                    guard.invalid_pending_hash = guard.invalid_pending_hash.saturating_add(1);
                }
            }
        }
    }

    pub fn observe_searcher_batch(&self, executable: &[OpportunityRecord]) {
        if let Ok(mut guard) = self.inner.searcher_metrics.write() {
            let executable_count = executable.len() as u64;

            guard.executable_batches_total = guard.executable_batches_total.saturating_add(1);
            guard.executable_candidates_total = guard
                .executable_candidates_total
                .saturating_add(executable_count);
            guard.executable_bundle_candidates_total =
                guard.executable_bundle_candidates_total.saturating_add(
                    executable
                        .iter()
                        .filter(|candidate| candidate.strategy == "BundleCandidate")
                        .count() as u64,
                );
            guard.max_executable_candidates_in_batch = guard
                .max_executable_candidates_in_batch
                .max(executable_count);

            let executable_top_score = executable
                .first()
                .map_or(0, |candidate| candidate.score as u64);
            guard.executable_top_score_total = guard
                .executable_top_score_total
                .saturating_add(executable_top_score);
            guard.executable_only_candidates_total = guard
                .executable_only_candidates_total
                .saturating_add(executable_count);
        }
    }

    pub fn configure_simulation_queue(&self, queue_capacity: usize, worker_total: usize) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.queue_capacity = queue_capacity as u64;
            guard.worker_total = worker_total as u64;
        }
    }

    pub fn observe_simulation_enqueue(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.enqueued_total = guard.enqueued_total.saturating_add(1);
            guard.queue_depth = guard.queue_depth.saturating_add(1);
        }
    }

    pub fn observe_simulation_dequeue(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.queue_depth = guard.queue_depth.saturating_sub(1);
            guard.inflight_current = guard.inflight_current.saturating_add(1);
        }
    }

    pub fn observe_simulation_finish(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.inflight_current = guard.inflight_current.saturating_sub(1);
        }
    }

    pub fn observe_simulation_queue_full_drop(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.queue_full_drop_total = guard.queue_full_drop_total.saturating_add(1);
        }
    }

    pub fn observe_simulation_stale_drop(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.stale_drop_total = guard.stale_drop_total.saturating_add(1);
        }
    }

    pub fn observe_simulation_cache_hit(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.cache_hit_total = guard.cache_hit_total.saturating_add(1);
        }
    }

    pub fn observe_simulation_cache_miss(&self) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.cache_miss_total = guard.cache_miss_total.saturating_add(1);
        }
    }

    pub fn observe_simulation_result(
        &self,
        status: &str,
        latency_ms: u64,
        tx_count: usize,
        fail_category: Option<&str>,
    ) {
        if let Ok(mut guard) = self.inner.simulation_metrics.write() {
            guard.completed_total = guard.completed_total.saturating_add(1);
            guard.tx_total = guard.tx_total.saturating_add(tx_count as u64);
            guard.last_latency_ms = latency_ms;
            guard.max_latency_ms = guard.max_latency_ms.max(latency_ms);
            guard.total_latency_ms = guard.total_latency_ms.saturating_add(latency_ms);

            match status {
                "ok" => {
                    guard.ok_total = guard.ok_total.saturating_add(1);
                }
                "failed" => {
                    guard.failed_total = guard.failed_total.saturating_add(1);
                }
                "state_error" => {
                    guard.state_error_total = guard.state_error_total.saturating_add(1);
                }
                "timeout" => {
                    guard.timeout_total = guard.timeout_total.saturating_add(1);
                }
                _ => {}
            }

            if let Some(category) = fail_category {
                match category {
                    "revert" => {
                        guard.revert_fail_total = guard.revert_fail_total.saturating_add(1);
                    }
                    "out_of_gas" => {
                        guard.out_of_gas_fail_total = guard.out_of_gas_fail_total.saturating_add(1);
                    }
                    "nonce_mismatch" => {
                        guard.nonce_mismatch_fail_total =
                            guard.nonce_mismatch_fail_total.saturating_add(1);
                    }
                    "state_mismatch" => {
                        guard.state_mismatch_fail_total =
                            guard.state_mismatch_fail_total.saturating_add(1);
                    }
                    "state_rpc" => {
                        guard.state_rpc_fail_total = guard.state_rpc_fail_total.saturating_add(1);
                    }
                    "state_timeout" => {
                        guard.state_timeout_fail_total =
                            guard.state_timeout_fail_total.saturating_add(1);
                    }
                    _ => {
                        guard.unknown_fail_total = guard.unknown_fail_total.saturating_add(1);
                    }
                }
            }
        }
    }

    pub fn record_simulation_status(&self, snapshot: LiveRpcSimulationStatusSnapshot) {
        if let Ok(mut guard) = self.inner.simulation_status.write() {
            guard.latest = Some(snapshot.clone());
            guard.by_id.insert(snapshot.id.clone(), snapshot);
        }
    }

    pub fn cached_account_seed(
        &self,
        http_url: &str,
        address: Address,
        block_number: u64,
        now_unix_ms: i64,
        ttl_ms: i64,
    ) -> Option<AccountSeed> {
        let cache = self.inner.simulation_cache.read().ok()?;
        let entry = cache
            .account_seeds
            .get(&(http_url.to_owned(), address))
            .copied()?;
        if entry.block_number != block_number {
            return None;
        }
        if now_unix_ms.saturating_sub(entry.cached_at_unix_ms) > ttl_ms {
            return None;
        }
        Some(entry.seed)
    }

    pub fn cache_account_seed(
        &self,
        http_url: &str,
        address: Address,
        block_number: u64,
        cached_at_unix_ms: i64,
        seed: AccountSeed,
    ) {
        if let Ok(mut cache) = self.inner.simulation_cache.write() {
            cache.account_seeds.insert(
                (http_url.to_owned(), address),
                CachedAccountSeed {
                    seed,
                    block_number,
                    cached_at_unix_ms,
                },
            );
        }
    }

    pub fn reset_chain_status(&self) {
        if let Ok(mut guard) = self.inner.chain_status.write() {
            guard.clear();
        }
    }

    pub fn upsert_chain_status(&self, status: LiveRpcChainStatus) {
        if let Ok(mut guard) = self.inner.chain_status.write() {
            guard.insert(status.chain_key.clone(), status);
        }
    }

    pub fn with_builder_engine_mut<R>(&self, f: impl FnOnce(&mut AssemblyEngine) -> R) -> R {
        let mut guard = self
            .inner
            .builder_engine
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        f(&mut guard)
    }

    pub async fn shutdown(self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct SimulationStatusStore {
    latest: Option<LiveRpcSimulationStatusSnapshot>,
    by_id: HashMap<String, LiveRpcSimulationStatusSnapshot>,
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeCore, RuntimeCoreConfig, RuntimeCoreDeps, RuntimeCoreStartArgs, RuntimeIngestMode,
    };
    use common::Address;
    use scheduler::{SchedulerConfig, scheduler_channel};
    use sim_engine::AccountSeed;
    use std::sync::{Arc, RwLock};
    use storage::{InMemoryStorage, StorageWriteHandle, StorageWriteOp};
    use tokio::sync::mpsc;

    #[test]
    fn runtime_core_starts_with_default_views() {
        let (scheduler, _runtime) = scheduler_channel(SchedulerConfig::default());
        let (storage_tx, _storage_rx) = mpsc::channel::<StorageWriteOp>(8);
        let args = RuntimeCoreStartArgs {
            deps: RuntimeCoreDeps {
                storage: Arc::new(RwLock::new(InMemoryStorage::default())),
                writer: StorageWriteHandle::from_sender(storage_tx),
                scheduler,
            },
            config: RuntimeCoreConfig {
                ingest_mode: RuntimeIngestMode::Rpc,
                rebuild_scheduler_from_rpc: false,
            },
        };

        let handle = RuntimeCore::start(args).expect("runtime core should start");

        assert_eq!(handle.builder_snapshot().candidates.len(), 0);
        assert_eq!(handle.builder_metrics().inserted_total, 0);
        assert_eq!(handle.searcher_metrics().executable_batches_total, 0);
        assert_eq!(handle.simulation_metrics().completed_total, 0);
        assert_eq!(handle.drop_metrics().storage_queue_full, 0);
        assert!(handle.chain_status().is_empty());
        assert_eq!(handle.simulation_status("latest"), None);
    }

    #[test]
    fn runtime_core_instances_do_not_share_builder_state() {
        let first = make_runtime_core();
        let second = make_runtime_core();

        first.with_builder_engine_mut(|engine| {
            let _ = engine.insert(builder::AssemblyCandidate {
                candidate_id: "cand-a".to_owned(),
                tx_hashes: vec![[0x11; 32]],
                priority_score: 100,
                gas_used: 21_000,
                kind: builder::AssemblyCandidateKind::Transaction,
                simulation: builder::SimulationApproval {
                    sim_id: "sim-a".to_owned(),
                    block_number: 1,
                    approved: true,
                },
            });
        });

        assert_eq!(first.builder_snapshot().candidates.len(), 1);
        assert!(second.builder_snapshot().candidates.is_empty());
    }

    #[test]
    fn runtime_core_instances_do_not_share_simulation_cache() {
        let first = make_runtime_core();
        let second = make_runtime_core();
        let sender: Address = [0x22; 20];
        let seed = AccountSeed {
            balance_wei: 123,
            nonce: 7,
        };

        first.cache_account_seed("http://rpc-a", sender, 100, 1_700_000_000_000, seed);

        assert_eq!(
            first.cached_account_seed("http://rpc-a", sender, 100, 1_700_000_000_100, 1_000),
            Some(seed)
        );
        assert_eq!(
            second.cached_account_seed("http://rpc-a", sender, 100, 1_700_000_000_100, 1_000),
            None
        );
    }

    fn make_runtime_core() -> super::RuntimeCoreHandle {
        let (scheduler, _runtime) = scheduler_channel(SchedulerConfig::default());
        let (storage_tx, _storage_rx) = mpsc::channel::<StorageWriteOp>(8);
        RuntimeCore::start(RuntimeCoreStartArgs {
            deps: RuntimeCoreDeps {
                storage: Arc::new(RwLock::new(InMemoryStorage::default())),
                writer: StorageWriteHandle::from_sender(storage_tx),
                scheduler,
            },
            config: RuntimeCoreConfig {
                ingest_mode: RuntimeIngestMode::Rpc,
                rebuild_scheduler_from_rpc: false,
            },
        })
        .expect("runtime core should start")
    }
}
