use ahash::RandomState;
use anyhow::{Context, Result, anyhow};
use builder::{
    AssemblyCandidate, AssemblyConfig, AssemblyDecision, AssemblyEngine, AssemblyMetrics,
    AssemblySnapshot,
};
use common::{Address, SourceId, TxHash};
use event_log::{
    BundleSubmitted, EventPayload, OppDetected, SimCompleted, TxBlocked, TxDecoded, TxDropped,
    TxFetched, TxReady, TxReplaced, TxSeen,
};
use feature_engine::{
    FeatureAnalysis, FeatureInput, analyze_transaction, version as feature_engine_version,
};
use futures::{SinkExt, StreamExt};
use hashbrown::{HashMap, HashSet};
use scheduler::{
    SchedulerAdmission, SchedulerEnqueueError, SchedulerHandle, SchedulerQueueState,
    SchedulerQueueTransition, ValidatedTransaction,
};
use searcher::{OpportunityCandidate, SearcherConfig, SearcherInputTx, rank_opportunity_batch};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::json;
use sim_engine::{
    AccountSeed, ChainContext, SimulationFailCategory, SimulationMode, SimulationTxInput,
    StateProvider, simulate_with_mode,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use storage::{
    EventStore, InMemoryStorage, OpportunityRecord, StorageTryEnqueueError, StorageWriteHandle,
    StorageWriteOp, TxFeaturesRecord, TxFullRecord, TxSeenRecord,
};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type FastSet<T> = HashSet<T, RandomState>;

const PRIMARY_PUBLIC_WS_URL: &str = "wss://eth.drpc.org";
const PRIMARY_PUBLIC_HTTP_URL: &str = "https://eth.drpc.org";
const FALLBACK_PUBLIC_WS_URL: &str = "wss://ethereum-rpc.publicnode.com";
const FALLBACK_PUBLIC_HTTP_URL: &str = "https://ethereum-rpc.publicnode.com";
const ENV_ETH_WS_URL: &str = "VIZ_API_ETH_WS_URL";
const ENV_ETH_HTTP_URL: &str = "VIZ_API_ETH_HTTP_URL";
const ENV_SOURCE_ID: &str = "VIZ_API_SOURCE_ID";
const ENV_CHAINS: &str = "VIZ_API_CHAINS";
const ENV_CHAIN_CONFIG_PATH: &str = "VIZ_API_CHAIN_CONFIG_PATH";
const DEFAULT_CHAIN_CONFIG_PATH: &str = "configs/chain_config.json";
const ENV_MAX_SEEN_HASHES: &str = "VIZ_API_MAX_SEEN_HASHES";
const ENV_RPC_BATCH_SIZE: &str = "VIZ_API_RPC_BATCH_SIZE";
const ENV_RPC_MAX_IN_FLIGHT: &str = "VIZ_API_RPC_MAX_IN_FLIGHT";
const ENV_RPC_RETRY_ATTEMPTS: &str = "VIZ_API_RPC_RETRY_ATTEMPTS";
const ENV_RPC_RETRY_BACKOFF_MS: &str = "VIZ_API_RPC_RETRY_BACKOFF_MS";
const ENV_RPC_BATCH_FLUSH_MS: &str = "VIZ_API_RPC_BATCH_FLUSH_MS";
const ENV_SILENT_CHAIN_TIMEOUT_SECS: &str = "VIZ_API_SILENT_CHAIN_TIMEOUT_SECS";
const ENV_SIM_CACHE_TTL_MS: &str = "VIZ_API_SIM_CACHE_TTL_MS";
const ENV_SIM_RPC_TIMEOUT_MS: &str = "VIZ_API_SIM_RPC_TIMEOUT_MS";
const ENV_SIM_QUEUE_CAPACITY: &str = "VIZ_API_SIM_QUEUE_CAPACITY";
const ENV_SIM_WORKER_COUNT: &str = "VIZ_API_SIM_WORKER_COUNT";
const DEFAULT_SILENT_CHAIN_TIMEOUT_SECS: u64 = 20;
const DEFAULT_SIM_CACHE_TTL_MS: u64 = 5_000;
const DEFAULT_SIM_RPC_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_SIM_QUEUE_CAPACITY: usize = 128;
const DEFAULT_SIM_WORKER_COUNT: usize = 4;
const SEARCHER_MIN_SCORE: u32 = 0;
const SEARCHER_MAX_CANDIDATES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LiveRpcDropMetricsSnapshot {
    pub storage_queue_full: u64,
    pub storage_queue_closed: u64,
    pub invalid_pending_hash: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LiveRpcSearcherMetricsSnapshot {
    pub executable_batches_total: u64,
    pub executable_candidates_total: u64,
    pub executable_bundle_candidates_total: u64,
    pub max_executable_candidates_in_batch: u64,
    pub legacy_shadow_batches_total: u64,
    pub legacy_shadow_candidates_total: u64,
    pub max_legacy_shadow_candidates_in_batch: u64,
    pub comparison_batches_total: u64,
    pub executable_top_score_total: u64,
    pub legacy_top_score_total: u64,
    pub executable_top_score_wins_total: u64,
    pub legacy_top_score_wins_total: u64,
    pub top_score_ties_total: u64,
    pub overlapping_candidates_total: u64,
    pub executable_only_candidates_total: u64,
    pub legacy_only_candidates_total: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
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

#[derive(Debug, Default)]
struct LiveRpcDropMetrics {
    storage_queue_full: AtomicU64,
    storage_queue_closed: AtomicU64,
    invalid_pending_hash: AtomicU64,
}

#[derive(Debug, Default)]
struct LiveRpcSearcherMetrics {
    executable_batches_total: AtomicU64,
    executable_candidates_total: AtomicU64,
    executable_bundle_candidates_total: AtomicU64,
    max_executable_candidates_in_batch: AtomicU64,
    legacy_shadow_batches_total: AtomicU64,
    legacy_shadow_candidates_total: AtomicU64,
    max_legacy_shadow_candidates_in_batch: AtomicU64,
    comparison_batches_total: AtomicU64,
    executable_top_score_total: AtomicU64,
    legacy_top_score_total: AtomicU64,
    executable_top_score_wins_total: AtomicU64,
    legacy_top_score_wins_total: AtomicU64,
    top_score_ties_total: AtomicU64,
    overlapping_candidates_total: AtomicU64,
    executable_only_candidates_total: AtomicU64,
    legacy_only_candidates_total: AtomicU64,
}

#[derive(Debug, Default)]
struct LiveRpcSimulationMetrics {
    enqueued_total: AtomicU64,
    completed_total: AtomicU64,
    ok_total: AtomicU64,
    failed_total: AtomicU64,
    state_error_total: AtomicU64,
    timeout_total: AtomicU64,
    queue_full_drop_total: AtomicU64,
    stale_drop_total: AtomicU64,
    queue_depth: AtomicU64,
    queue_capacity: AtomicU64,
    inflight_current: AtomicU64,
    worker_total: AtomicU64,
    cache_hit_total: AtomicU64,
    cache_miss_total: AtomicU64,
    tx_total: AtomicU64,
    last_latency_ms: AtomicU64,
    max_latency_ms: AtomicU64,
    total_latency_ms: AtomicU64,
    revert_fail_total: AtomicU64,
    out_of_gas_fail_total: AtomicU64,
    nonce_mismatch_fail_total: AtomicU64,
    state_mismatch_fail_total: AtomicU64,
    unknown_fail_total: AtomicU64,
    state_rpc_fail_total: AtomicU64,
    state_timeout_fail_total: AtomicU64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutableOpportunity {
    record: OpportunityRecord,
    candidate: OpportunityCandidate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteSimulationRequest {
    tx_hash: TxHash,
    member_tx_hashes: Vec<TxHash>,
    txs: Vec<ValidatedTransaction>,
    detected_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteSimulationOutcome {
    sim_id: String,
    status: RemoteSimulationStatus,
    fail_category: Option<String>,
    latency_ms: u64,
    tx_count: u32,
    simulation_batch: Option<sim_engine::SimulationBatchResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteSimulationStatus {
    Ok,
    Failed,
    StateError,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CachedAccountSeed {
    seed: AccountSeed,
    block_number: u64,
    cached_at_unix_ms: i64,
}

#[derive(Clone)]
struct SimulationTask {
    task_key: String,
    generation: u64,
    chain: ChainRpcConfig,
    request: RemoteSimulationRequest,
    opportunities: Vec<ExecutableOpportunity>,
    writer: StorageWriteHandle,
    next_seq_id: Arc<AtomicU64>,
    enqueued_unix_ms: i64,
}

#[derive(Clone)]
struct LiveRpcSimulationService {
    runtime: Arc<RwLock<LiveRpcSimulationRuntime>>,
    queue_capacity: usize,
    worker_total: usize,
}

#[derive(Clone)]
struct LiveRpcSimulationRuntime {
    ingress_tx: mpsc::Sender<SimulationTask>,
    state: Arc<RwLock<LiveRpcSimulationServiceState>>,
}

#[derive(Default)]
struct LiveRpcSimulationServiceState {
    task_generations: HashMap<String, u64>,
    task_members: HashMap<String, Vec<TxHash>>,
    tasks_by_member: HashMap<TxHash, FastSet<String>>,
}

#[derive(Default)]
struct LiveRpcSimulationStatusStore {
    latest: Option<LiveRpcSimulationStatusSnapshot>,
    by_id: HashMap<String, LiveRpcSimulationStatusSnapshot>,
}

#[derive(Clone, Debug, Default)]
struct RemoteStateCache {
    account_seeds: HashMap<(String, Address), CachedAccountSeed>,
}

#[derive(Debug, Deserialize)]
struct RpcHeader {
    number: String,
    timestamp: String,
    #[serde(rename = "gasLimit")]
    gas_limit: String,
    #[serde(default, rename = "baseFeePerGas")]
    base_fee_per_gas: Option<String>,
    #[serde(default)]
    miner: Option<String>,
    #[serde(default, rename = "stateRoot")]
    state_root: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcHeaderResponseEnvelope {
    #[serde(default)]
    error: Option<RpcFetchErrorEnvelope>,
    #[serde(default)]
    result: Option<RpcHeader>,
}

#[derive(Debug, Deserialize)]
struct RpcScalarResponseEnvelope {
    #[serde(default)]
    error: Option<RpcFetchErrorEnvelope>,
    #[serde(default)]
    result: Option<String>,
}

impl LiveRpcDropMetrics {
    fn observe(&self, reason: LiveRpcDropReason) {
        match reason {
            LiveRpcDropReason::StorageQueueFull => {
                self.storage_queue_full.fetch_add(1, Ordering::Relaxed);
            }
            LiveRpcDropReason::StorageQueueClosed => {
                self.storage_queue_closed.fetch_add(1, Ordering::Relaxed);
            }
            LiveRpcDropReason::InvalidPendingHash => {
                self.invalid_pending_hash.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn reset(&self) {
        self.storage_queue_full.store(0, Ordering::Relaxed);
        self.storage_queue_closed.store(0, Ordering::Relaxed);
        self.invalid_pending_hash.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> LiveRpcDropMetricsSnapshot {
        LiveRpcDropMetricsSnapshot {
            storage_queue_full: self.storage_queue_full.load(Ordering::Relaxed),
            storage_queue_closed: self.storage_queue_closed.load(Ordering::Relaxed),
            invalid_pending_hash: self.invalid_pending_hash.load(Ordering::Relaxed),
        }
    }
}

impl LiveRpcSearcherMetrics {
    fn observe_batch(&self, executable: &[OpportunityRecord], legacy_shadow: &[OpportunityRecord]) {
        let executable_count = executable.len() as u64;
        let legacy_count = legacy_shadow.len() as u64;

        self.executable_batches_total
            .fetch_add(1, Ordering::Relaxed);
        self.executable_candidates_total
            .fetch_add(executable_count, Ordering::Relaxed);
        self.executable_bundle_candidates_total.fetch_add(
            executable
                .iter()
                .filter(|candidate| candidate.strategy == "BundleCandidate")
                .count() as u64,
            Ordering::Relaxed,
        );
        self.max_executable_candidates_in_batch
            .fetch_max(executable_count, Ordering::Relaxed);

        self.legacy_shadow_batches_total
            .fetch_add(1, Ordering::Relaxed);
        self.legacy_shadow_candidates_total
            .fetch_add(legacy_count, Ordering::Relaxed);
        self.max_legacy_shadow_candidates_in_batch
            .fetch_max(legacy_count, Ordering::Relaxed);

        self.comparison_batches_total
            .fetch_add(1, Ordering::Relaxed);

        let executable_top_score = executable
            .first()
            .map_or(0, |candidate| candidate.score as u64);
        let legacy_top_score = legacy_shadow
            .first()
            .map_or(0, |candidate| candidate.score as u64);
        self.executable_top_score_total
            .fetch_add(executable_top_score, Ordering::Relaxed);
        self.legacy_top_score_total
            .fetch_add(legacy_top_score, Ordering::Relaxed);

        match executable_top_score.cmp(&legacy_top_score) {
            std::cmp::Ordering::Greater => {
                self.executable_top_score_wins_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            std::cmp::Ordering::Less => {
                self.legacy_top_score_wins_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            std::cmp::Ordering::Equal => {
                self.top_score_ties_total.fetch_add(1, Ordering::Relaxed);
            }
        }

        let executable_keys = executable
            .iter()
            .map(opportunity_identity)
            .collect::<BTreeSet<_>>();
        let legacy_keys = legacy_shadow
            .iter()
            .map(opportunity_identity)
            .collect::<BTreeSet<_>>();
        let overlapping = executable_keys.intersection(&legacy_keys).count() as u64;
        let executable_only = executable_keys.difference(&legacy_keys).count() as u64;
        let legacy_only = legacy_keys.difference(&executable_keys).count() as u64;

        self.overlapping_candidates_total
            .fetch_add(overlapping, Ordering::Relaxed);
        self.executable_only_candidates_total
            .fetch_add(executable_only, Ordering::Relaxed);
        self.legacy_only_candidates_total
            .fetch_add(legacy_only, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn reset(&self) {
        self.executable_batches_total.store(0, Ordering::Relaxed);
        self.executable_candidates_total.store(0, Ordering::Relaxed);
        self.executable_bundle_candidates_total
            .store(0, Ordering::Relaxed);
        self.max_executable_candidates_in_batch
            .store(0, Ordering::Relaxed);
        self.legacy_shadow_batches_total.store(0, Ordering::Relaxed);
        self.legacy_shadow_candidates_total
            .store(0, Ordering::Relaxed);
        self.max_legacy_shadow_candidates_in_batch
            .store(0, Ordering::Relaxed);
        self.comparison_batches_total.store(0, Ordering::Relaxed);
        self.executable_top_score_total.store(0, Ordering::Relaxed);
        self.legacy_top_score_total.store(0, Ordering::Relaxed);
        self.executable_top_score_wins_total
            .store(0, Ordering::Relaxed);
        self.legacy_top_score_wins_total.store(0, Ordering::Relaxed);
        self.top_score_ties_total.store(0, Ordering::Relaxed);
        self.overlapping_candidates_total
            .store(0, Ordering::Relaxed);
        self.executable_only_candidates_total
            .store(0, Ordering::Relaxed);
        self.legacy_only_candidates_total
            .store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> LiveRpcSearcherMetricsSnapshot {
        LiveRpcSearcherMetricsSnapshot {
            executable_batches_total: self.executable_batches_total.load(Ordering::Relaxed),
            executable_candidates_total: self.executable_candidates_total.load(Ordering::Relaxed),
            executable_bundle_candidates_total: self
                .executable_bundle_candidates_total
                .load(Ordering::Relaxed),
            max_executable_candidates_in_batch: self
                .max_executable_candidates_in_batch
                .load(Ordering::Relaxed),
            legacy_shadow_batches_total: self.legacy_shadow_batches_total.load(Ordering::Relaxed),
            legacy_shadow_candidates_total: self
                .legacy_shadow_candidates_total
                .load(Ordering::Relaxed),
            max_legacy_shadow_candidates_in_batch: self
                .max_legacy_shadow_candidates_in_batch
                .load(Ordering::Relaxed),
            comparison_batches_total: self.comparison_batches_total.load(Ordering::Relaxed),
            executable_top_score_total: self.executable_top_score_total.load(Ordering::Relaxed),
            legacy_top_score_total: self.legacy_top_score_total.load(Ordering::Relaxed),
            executable_top_score_wins_total: self
                .executable_top_score_wins_total
                .load(Ordering::Relaxed),
            legacy_top_score_wins_total: self.legacy_top_score_wins_total.load(Ordering::Relaxed),
            top_score_ties_total: self.top_score_ties_total.load(Ordering::Relaxed),
            overlapping_candidates_total: self.overlapping_candidates_total.load(Ordering::Relaxed),
            executable_only_candidates_total: self
                .executable_only_candidates_total
                .load(Ordering::Relaxed),
            legacy_only_candidates_total: self.legacy_only_candidates_total.load(Ordering::Relaxed),
        }
    }
}

impl LiveRpcSimulationMetrics {
    fn configure_queue(&self, queue_capacity: usize, worker_total: usize) {
        self.queue_capacity
            .store(queue_capacity as u64, Ordering::Relaxed);
        self.worker_total
            .store(worker_total as u64, Ordering::Relaxed);
    }

    fn observe_enqueue(&self) {
        self.enqueued_total.fetch_add(1, Ordering::Relaxed);
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
    }

    fn observe_dequeue(&self) {
        self.queue_depth.fetch_sub(1, Ordering::Relaxed);
        self.inflight_current.fetch_add(1, Ordering::Relaxed);
    }

    fn observe_finish(&self) {
        self.inflight_current.fetch_sub(1, Ordering::Relaxed);
    }

    fn observe_queue_full_drop(&self) {
        self.queue_full_drop_total.fetch_add(1, Ordering::Relaxed);
    }

    fn observe_stale_drop(&self) {
        self.stale_drop_total.fetch_add(1, Ordering::Relaxed);
    }

    fn observe_cache_hit(&self) {
        self.cache_hit_total.fetch_add(1, Ordering::Relaxed);
    }

    fn observe_cache_miss(&self) {
        self.cache_miss_total.fetch_add(1, Ordering::Relaxed);
    }

    fn observe_result(
        &self,
        status: RemoteSimulationStatus,
        latency_ms: u64,
        tx_count: usize,
        fail_category: Option<&str>,
    ) {
        self.completed_total.fetch_add(1, Ordering::Relaxed);
        self.tx_total.fetch_add(tx_count as u64, Ordering::Relaxed);
        self.last_latency_ms.store(latency_ms, Ordering::Relaxed);
        self.max_latency_ms.fetch_max(latency_ms, Ordering::Relaxed);
        self.total_latency_ms
            .fetch_add(latency_ms, Ordering::Relaxed);

        match status {
            RemoteSimulationStatus::Ok => {
                self.ok_total.fetch_add(1, Ordering::Relaxed);
            }
            RemoteSimulationStatus::Failed => {
                self.failed_total.fetch_add(1, Ordering::Relaxed);
            }
            RemoteSimulationStatus::StateError => {
                self.state_error_total.fetch_add(1, Ordering::Relaxed);
            }
            RemoteSimulationStatus::Timeout => {
                self.timeout_total.fetch_add(1, Ordering::Relaxed);
            }
        }

        if let Some(category) = fail_category {
            match category {
                "revert" => {
                    self.revert_fail_total.fetch_add(1, Ordering::Relaxed);
                }
                "out_of_gas" => {
                    self.out_of_gas_fail_total.fetch_add(1, Ordering::Relaxed);
                }
                "nonce_mismatch" => {
                    self.nonce_mismatch_fail_total
                        .fetch_add(1, Ordering::Relaxed);
                }
                "state_mismatch" => {
                    self.state_mismatch_fail_total
                        .fetch_add(1, Ordering::Relaxed);
                }
                "state_rpc" => {
                    self.state_rpc_fail_total.fetch_add(1, Ordering::Relaxed);
                }
                "state_timeout" => {
                    self.state_timeout_fail_total
                        .fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    self.unknown_fail_total.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        self.enqueued_total.store(0, Ordering::Relaxed);
        self.completed_total.store(0, Ordering::Relaxed);
        self.ok_total.store(0, Ordering::Relaxed);
        self.failed_total.store(0, Ordering::Relaxed);
        self.state_error_total.store(0, Ordering::Relaxed);
        self.timeout_total.store(0, Ordering::Relaxed);
        self.queue_full_drop_total.store(0, Ordering::Relaxed);
        self.stale_drop_total.store(0, Ordering::Relaxed);
        self.queue_depth.store(0, Ordering::Relaxed);
        self.inflight_current.store(0, Ordering::Relaxed);
        self.cache_hit_total.store(0, Ordering::Relaxed);
        self.cache_miss_total.store(0, Ordering::Relaxed);
        self.tx_total.store(0, Ordering::Relaxed);
        self.last_latency_ms.store(0, Ordering::Relaxed);
        self.max_latency_ms.store(0, Ordering::Relaxed);
        self.total_latency_ms.store(0, Ordering::Relaxed);
        self.revert_fail_total.store(0, Ordering::Relaxed);
        self.out_of_gas_fail_total.store(0, Ordering::Relaxed);
        self.nonce_mismatch_fail_total.store(0, Ordering::Relaxed);
        self.state_mismatch_fail_total.store(0, Ordering::Relaxed);
        self.unknown_fail_total.store(0, Ordering::Relaxed);
        self.state_rpc_fail_total.store(0, Ordering::Relaxed);
        self.state_timeout_fail_total.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self) -> LiveRpcSimulationMetricsSnapshot {
        LiveRpcSimulationMetricsSnapshot {
            enqueued_total: self.enqueued_total.load(Ordering::Relaxed),
            completed_total: self.completed_total.load(Ordering::Relaxed),
            ok_total: self.ok_total.load(Ordering::Relaxed),
            failed_total: self.failed_total.load(Ordering::Relaxed),
            state_error_total: self.state_error_total.load(Ordering::Relaxed),
            timeout_total: self.timeout_total.load(Ordering::Relaxed),
            queue_full_drop_total: self.queue_full_drop_total.load(Ordering::Relaxed),
            stale_drop_total: self.stale_drop_total.load(Ordering::Relaxed),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            queue_capacity: self.queue_capacity.load(Ordering::Relaxed),
            inflight_current: self.inflight_current.load(Ordering::Relaxed),
            worker_total: self.worker_total.load(Ordering::Relaxed),
            cache_hit_total: self.cache_hit_total.load(Ordering::Relaxed),
            cache_miss_total: self.cache_miss_total.load(Ordering::Relaxed),
            tx_total: self.tx_total.load(Ordering::Relaxed),
            last_latency_ms: self.last_latency_ms.load(Ordering::Relaxed),
            max_latency_ms: self.max_latency_ms.load(Ordering::Relaxed),
            total_latency_ms: self.total_latency_ms.load(Ordering::Relaxed),
            revert_fail_total: self.revert_fail_total.load(Ordering::Relaxed),
            out_of_gas_fail_total: self.out_of_gas_fail_total.load(Ordering::Relaxed),
            nonce_mismatch_fail_total: self.nonce_mismatch_fail_total.load(Ordering::Relaxed),
            state_mismatch_fail_total: self.state_mismatch_fail_total.load(Ordering::Relaxed),
            unknown_fail_total: self.unknown_fail_total.load(Ordering::Relaxed),
            state_rpc_fail_total: self.state_rpc_fail_total.load(Ordering::Relaxed),
            state_timeout_fail_total: self.state_timeout_fail_total.load(Ordering::Relaxed),
        }
    }
}

impl LiveRpcSimulationServiceState {
    fn current_generation(&self, task_key: &str) -> u64 {
        self.task_generations.get(task_key).copied().unwrap_or(0)
    }

    fn replace_membership(&mut self, task_key: &str, members: &[TxHash]) {
        if let Some(previous) = self
            .task_members
            .insert(task_key.to_owned(), members.to_vec())
        {
            for member in previous {
                if let Some(keys) = self.tasks_by_member.get_mut(&member) {
                    keys.remove(task_key);
                    if keys.is_empty() {
                        self.tasks_by_member.remove(&member);
                    }
                }
            }
        }

        for member in members {
            self.tasks_by_member
                .entry(*member)
                .or_insert_with(FastSet::default)
                .insert(task_key.to_owned());
        }
    }

    fn commit_enqueue(&mut self, task_key: &str, generation: u64, members: &[TxHash]) {
        self.task_generations
            .insert(task_key.to_owned(), generation);
        self.replace_membership(task_key, members);
    }

    fn is_current(&self, task_key: &str, generation: u64) -> bool {
        self.current_generation(task_key) == generation
    }

    fn invalidate_hash(&mut self, hash: TxHash) {
        let Some(task_keys) = self.tasks_by_member.remove(&hash) else {
            return;
        };

        for task_key in task_keys {
            let next_generation = self.current_generation(&task_key).saturating_add(1).max(1);
            self.task_generations
                .insert(task_key.clone(), next_generation);

            if let Some(previous_members) = self.task_members.remove(&task_key) {
                for member in previous_members {
                    if member == hash {
                        continue;
                    }

                    if let Some(keys) = self.tasks_by_member.get_mut(&member) {
                        keys.remove(&task_key);
                        if keys.is_empty() {
                            self.tasks_by_member.remove(&member);
                        }
                    }
                }
            }
        }
    }

    fn complete_current(&mut self, task_key: &str, generation: u64) {
        if !self.is_current(task_key, generation) {
            return;
        }

        self.task_generations.remove(task_key);
        if let Some(previous_members) = self.task_members.remove(task_key) {
            for member in previous_members {
                if let Some(keys) = self.tasks_by_member.get_mut(&member) {
                    keys.remove(task_key);
                    if keys.is_empty() {
                        self.tasks_by_member.remove(&member);
                    }
                }
            }
        }
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.task_generations.clear();
        self.task_members.clear();
        self.tasks_by_member.clear();
    }
}

static LIVE_RPC_DROP_METRICS: OnceLock<Arc<LiveRpcDropMetrics>> = OnceLock::new();
static LIVE_RPC_SEARCHER_METRICS: OnceLock<Arc<LiveRpcSearcherMetrics>> = OnceLock::new();
static LIVE_RPC_SIMULATION_METRICS: OnceLock<Arc<LiveRpcSimulationMetrics>> = OnceLock::new();
static LIVE_RPC_SIMULATION_CACHE: OnceLock<Arc<RwLock<RemoteStateCache>>> = OnceLock::new();
static LIVE_RPC_SIMULATION_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static LIVE_RPC_SIMULATION_SERVICE: OnceLock<LiveRpcSimulationService> = OnceLock::new();
static LIVE_RPC_SIMULATION_STATUS: OnceLock<Arc<RwLock<LiveRpcSimulationStatusStore>>> =
    OnceLock::new();
static LIVE_RPC_BUILDER_ENGINE: OnceLock<Arc<RwLock<AssemblyEngine>>> = OnceLock::new();
static LIVE_RPC_FEED_START_COUNT: AtomicU64 = AtomicU64::new(0);
static LIVE_RPC_MONO_EPOCH: OnceLock<Instant> = OnceLock::new();

fn live_rpc_drop_metrics() -> &'static Arc<LiveRpcDropMetrics> {
    LIVE_RPC_DROP_METRICS.get_or_init(|| Arc::new(LiveRpcDropMetrics::default()))
}

fn live_rpc_searcher_metrics() -> &'static Arc<LiveRpcSearcherMetrics> {
    LIVE_RPC_SEARCHER_METRICS.get_or_init(|| Arc::new(LiveRpcSearcherMetrics::default()))
}

fn live_rpc_simulation_metrics() -> &'static Arc<LiveRpcSimulationMetrics> {
    LIVE_RPC_SIMULATION_METRICS.get_or_init(|| Arc::new(LiveRpcSimulationMetrics::default()))
}

fn live_rpc_simulation_cache() -> &'static Arc<RwLock<RemoteStateCache>> {
    LIVE_RPC_SIMULATION_CACHE.get_or_init(|| Arc::new(RwLock::new(RemoteStateCache::default())))
}

fn simulation_http_client() -> &'static reqwest::Client {
    LIVE_RPC_SIMULATION_HTTP_CLIENT.get_or_init(reqwest::Client::new)
}

fn live_rpc_simulation_status_store() -> &'static Arc<RwLock<LiveRpcSimulationStatusStore>> {
    LIVE_RPC_SIMULATION_STATUS
        .get_or_init(|| Arc::new(RwLock::new(LiveRpcSimulationStatusStore::default())))
}

fn live_rpc_builder_engine() -> &'static Arc<RwLock<AssemblyEngine>> {
    LIVE_RPC_BUILDER_ENGINE
        .get_or_init(|| Arc::new(RwLock::new(AssemblyEngine::new(AssemblyConfig::default()))))
}

fn spawn_live_rpc_simulation_runtime(
    queue_capacity: usize,
    worker_total: usize,
) -> LiveRpcSimulationRuntime {
    let (ingress_tx, ingress_rx) = mpsc::channel(queue_capacity);
    let receiver = Arc::new(Mutex::new(ingress_rx));
    let state = Arc::new(RwLock::new(LiveRpcSimulationServiceState::default()));

    for _ in 0..worker_total {
        tokio::spawn(run_simulation_worker(receiver.clone(), state.clone()));
    }

    LiveRpcSimulationRuntime { ingress_tx, state }
}

fn live_rpc_simulation_service() -> &'static LiveRpcSimulationService {
    LIVE_RPC_SIMULATION_SERVICE.get_or_init(|| {
        let queue_capacity = resolve_sim_queue_capacity();
        let worker_total = resolve_sim_worker_count();
        live_rpc_simulation_metrics().configure_queue(queue_capacity, worker_total);
        LiveRpcSimulationService {
            runtime: Arc::new(RwLock::new(spawn_live_rpc_simulation_runtime(
                queue_capacity,
                worker_total,
            ))),
            queue_capacity,
            worker_total,
        }
    })
}

impl LiveRpcSimulationService {
    fn runtime(&self) -> LiveRpcSimulationRuntime {
        {
            let runtime = self
                .runtime
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            if !runtime.ingress_tx.is_closed() {
                return runtime.clone();
            }
        }

        let mut runtime = self
            .runtime
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        if runtime.ingress_tx.is_closed() {
            *runtime = spawn_live_rpc_simulation_runtime(self.queue_capacity, self.worker_total);
        }
        runtime.clone()
    }

    fn enqueue(&self, mut task: SimulationTask) -> Result<()> {
        let runtime = self.runtime();
        let task_key = task.task_key.clone();
        let next_generation = {
            let state = runtime
                .state
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            state.current_generation(&task_key).saturating_add(1).max(1)
        };
        task.generation = next_generation;
        let members = task.request.member_tx_hashes.clone();
        match runtime.ingress_tx.try_send(task) {
            Ok(()) => {
                let mut state = runtime
                    .state
                    .write()
                    .unwrap_or_else(|poison| poison.into_inner());
                state.commit_enqueue(&task_key, next_generation, &members);
                live_rpc_simulation_metrics().observe_enqueue();
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                live_rpc_simulation_metrics().observe_queue_full_drop();
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(anyhow!("live rpc simulation queue closed"))
            }
        }
    }

    fn invalidate_hash(&self, hash: TxHash) {
        let runtime = self.runtime();
        let mut state = runtime
            .state
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        state.invalidate_hash(hash);
    }
}

fn resolve_sim_queue_capacity() -> usize {
    match parse_env_usize(ENV_SIM_QUEUE_CAPACITY) {
        Ok(Some(value)) => value.max(1),
        Ok(None) => DEFAULT_SIM_QUEUE_CAPACITY,
        Err(err) => {
            tracing::warn!(error = %err, "invalid simulation queue capacity override; using default");
            DEFAULT_SIM_QUEUE_CAPACITY
        }
    }
}

fn resolve_sim_worker_count() -> usize {
    match parse_env_usize(ENV_SIM_WORKER_COUNT) {
        Ok(Some(value)) => value.max(1),
        Ok(None) => DEFAULT_SIM_WORKER_COUNT,
        Err(err) => {
            tracing::warn!(error = %err, "invalid simulation worker count override; using default");
            DEFAULT_SIM_WORKER_COUNT
        }
    }
}

async fn run_simulation_worker(
    receiver: Arc<Mutex<mpsc::Receiver<SimulationTask>>>,
    state: Arc<RwLock<LiveRpcSimulationServiceState>>,
) {
    loop {
        let task = {
            let mut receiver = receiver.lock().await;
            receiver.recv().await
        };
        let Some(task) = task else {
            break;
        };

        live_rpc_simulation_metrics().observe_dequeue();

        let is_current = {
            let state = state.read().unwrap_or_else(|poison| poison.into_inner());
            state.is_current(&task.task_key, task.generation)
        };
        if !is_current {
            live_rpc_simulation_metrics().observe_stale_drop();
            live_rpc_simulation_metrics().observe_finish();
            continue;
        }

        let started_unix_ms = current_unix_ms();
        let outcome =
            simulate_remote_request(simulation_http_client(), &task.chain, &task.request).await;
        let finished_unix_ms = current_unix_ms();

        let still_current = {
            let state = state.read().unwrap_or_else(|poison| poison.into_inner());
            state.is_current(&task.task_key, task.generation)
        };
        if !still_current {
            live_rpc_simulation_metrics().observe_stale_drop();
            live_rpc_simulation_metrics().observe_finish();
            continue;
        }

        match outcome {
            Ok(outcome) => {
                if let Err(err) = apply_simulation_task_outcome(
                    &task,
                    &outcome,
                    started_unix_ms,
                    finished_unix_ms,
                ) {
                    tracing::warn!(
                        error = %err,
                        chain_key = %task.chain.chain_key,
                        tx_hash = %format_fixed_hex(&task.request.tx_hash),
                        "failed to persist live rpc simulation result"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    chain_key = %task.chain.chain_key,
                    tx_hash = %format_fixed_hex(&task.request.tx_hash),
                    "live rpc simulation worker failed before producing a result"
                );
            }
        }

        {
            let mut state = state.write().unwrap_or_else(|poison| poison.into_inner());
            state.complete_current(&task.task_key, task.generation);
        }
        live_rpc_simulation_metrics().observe_finish();
    }
}

fn apply_simulation_task_outcome(
    task: &SimulationTask,
    outcome: &RemoteSimulationOutcome,
    started_unix_ms: i64,
    finished_unix_ms: i64,
) -> Result<()> {
    apply_builder_assembly_state(task, outcome);
    record_live_rpc_simulation_status(build_live_rpc_simulation_status_snapshot(
        task,
        outcome,
        started_unix_ms,
        finished_unix_ms,
    ));

    for opportunity in &task.opportunities {
        let sim_event = build_sim_completed_payload(&opportunity.record, outcome);
        if !append_event(
            &task.writer,
            &task.chain,
            &task.next_seq_id,
            finished_unix_ms,
            EventPayload::SimCompleted(sim_event.clone()),
        )? {
            return Ok(());
        }

        if outcome.status == RemoteSimulationStatus::Ok {
            if !append_event(
                &task.writer,
                &task.chain,
                &task.next_seq_id,
                finished_unix_ms,
                build_bundle_submitted_payload(&opportunity.record, &sim_event),
            )? {
                return Ok(());
            }
            if !try_enqueue_storage_write(
                &task.writer,
                &task.chain,
                StorageWriteOp::UpsertOpportunity(opportunity.record.clone()),
            )? {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn build_live_rpc_simulation_status_snapshot(
    task: &SimulationTask,
    outcome: &RemoteSimulationOutcome,
    _started_unix_ms: i64,
    finished_unix_ms: i64,
) -> LiveRpcSimulationStatusSnapshot {
    let bundle_id = task
        .opportunities
        .first()
        .map(|opportunity| bundle_id_for_opportunity(&opportunity.record))
        .unwrap_or_else(|| build_bundle_id(task.request.tx_hash, task.request.detected_unix_ms));
    LiveRpcSimulationStatusSnapshot {
        id: outcome.sim_id.clone(),
        bundle_id,
        status: simulation_status_label(outcome.status).to_owned(),
        relay_url: "not_submitted".to_owned(),
        attempt_count: 0,
        accepted: outcome.status == RemoteSimulationStatus::Ok,
        fail_category: outcome.fail_category.clone(),
        started_unix_ms: task.enqueued_unix_ms,
        finished_unix_ms,
    }
}

fn apply_builder_assembly_state(task: &SimulationTask, outcome: &RemoteSimulationOutcome) {
    let Some(batch) = outcome.simulation_batch.as_ref() else {
        return;
    };

    let mut engine = live_rpc_builder_engine()
        .write()
        .unwrap_or_else(|poison| poison.into_inner());
    for opportunity in &task.opportunities {
        match AssemblyCandidate::from_simulated_opportunity(&opportunity.candidate, batch) {
            Ok(candidate) => {
                let simulation_block_number = candidate.simulation.block_number;
                match engine.insert(candidate) {
                    AssemblyDecision::Inserted {
                        candidate_id,
                        replaced_candidate_ids,
                    } => {
                        tracing::debug!(
                            candidate_id,
                            tx_hash = %format_fixed_hex(&opportunity.record.tx_hash),
                            simulation_block_number,
                            replaced_candidate_ids = ?replaced_candidate_ids,
                            "builder candidate inserted into live assembly state"
                        );
                    }
                    AssemblyDecision::Rejected {
                        candidate_id,
                        reason,
                    } => {
                        tracing::debug!(
                            candidate_id,
                            tx_hash = %format_fixed_hex(&opportunity.record.tx_hash),
                            simulation_block_number,
                            reason,
                            "builder candidate rejected from live assembly state"
                        );
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = ?err,
                    tx_hash = %format_fixed_hex(&opportunity.record.tx_hash),
                    "failed to materialize builder candidate from simulation result"
                );
            }
        }
    }
}

fn record_live_rpc_simulation_status(snapshot: LiveRpcSimulationStatusSnapshot) {
    let mut store = live_rpc_simulation_status_store()
        .write()
        .unwrap_or_else(|poison| poison.into_inner());
    store.latest = Some(snapshot.clone());
    store.by_id.insert(snapshot.id.clone(), snapshot);
}

#[cfg(test)]
pub(crate) fn seed_live_rpc_simulation_status(snapshot: LiveRpcSimulationStatusSnapshot) {
    record_live_rpc_simulation_status(snapshot);
}

pub fn live_rpc_drop_metrics_snapshot() -> LiveRpcDropMetricsSnapshot {
    live_rpc_drop_metrics().snapshot()
}

pub fn reset_live_rpc_drop_metrics() {
    live_rpc_drop_metrics().reset();
}

pub fn live_rpc_searcher_metrics_snapshot() -> LiveRpcSearcherMetricsSnapshot {
    live_rpc_searcher_metrics().snapshot()
}

pub fn live_rpc_simulation_metrics_snapshot() -> LiveRpcSimulationMetricsSnapshot {
    live_rpc_simulation_metrics().snapshot()
}

pub fn live_rpc_simulation_status_snapshot(id: &str) -> Option<LiveRpcSimulationStatusSnapshot> {
    let store = live_rpc_simulation_status_store()
        .read()
        .unwrap_or_else(|poison| poison.into_inner());
    if id == "latest" {
        return store.latest.clone();
    }
    store.by_id.get(id).cloned()
}

pub fn live_rpc_builder_snapshot() -> AssemblySnapshot {
    live_rpc_builder_engine()
        .read()
        .map(|guard| guard.snapshot())
        .unwrap_or_default()
}

pub fn live_rpc_builder_metrics_snapshot() -> AssemblyMetrics {
    live_rpc_builder_engine()
        .read()
        .map(|guard| guard.metrics())
        .unwrap_or_default()
}

#[cfg(test)]
fn reset_live_rpc_searcher_metrics() {
    live_rpc_searcher_metrics().reset();
}

#[cfg(test)]
fn reset_live_rpc_simulation_metrics() {
    live_rpc_simulation_metrics().reset();
}

#[cfg(test)]
fn reset_live_rpc_simulation_cache() {
    let mut cache = live_rpc_simulation_cache()
        .write()
        .unwrap_or_else(|poison| poison.into_inner());
    cache.account_seeds.clear();
}

#[cfg(test)]
pub(crate) fn reset_live_rpc_simulation_runtime_state() {
    if let Some(service) = LIVE_RPC_SIMULATION_SERVICE.get()
        && let Ok(runtime) = service.runtime.read()
        && let Ok(mut state) = runtime.state.write()
    {
        state.clear();
    }
    if let Ok(mut store) = live_rpc_simulation_status_store().write() {
        store.latest = None;
        store.by_id.clear();
    }
    if let Ok(mut engine) = live_rpc_builder_engine().write() {
        *engine = AssemblyEngine::new(AssemblyConfig::default());
    }
}

#[cfg(test)]
pub(crate) fn reset_live_rpc_builder_runtime_state() {
    if let Ok(mut engine) = live_rpc_builder_engine().write() {
        *engine = AssemblyEngine::new(AssemblyConfig::default());
    }
}

pub fn observe_live_rpc_drop_reason(reason: LiveRpcDropReason) {
    live_rpc_drop_metrics().observe(reason);
}

pub fn live_rpc_feed_start_count() -> u64 {
    LIVE_RPC_FEED_START_COUNT.load(Ordering::Relaxed)
}

pub fn reset_live_rpc_feed_start_count() {
    LIVE_RPC_FEED_START_COUNT.store(0, Ordering::Relaxed);
}

fn current_mono_ns() -> u64 {
    LIVE_RPC_MONO_EPOCH
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

pub fn classify_storage_enqueue_drop_reason(error: StorageTryEnqueueError) -> LiveRpcDropReason {
    match error {
        StorageTryEnqueueError::QueueFull => LiveRpcDropReason::StorageQueueFull,
        StorageTryEnqueueError::QueueClosed => LiveRpcDropReason::StorageQueueClosed,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug)]
struct LiveRpcChainStatusRecord {
    chain_key: String,
    chain_id: Option<u64>,
    source_id: String,
    state: String,
    endpoint_index: usize,
    endpoint_count: usize,
    ws_url: String,
    http_url: String,
    last_pending_unix_ms: Option<i64>,
    updated_unix_ms: i64,
    last_error: Option<String>,
    rotation_count: u64,
}

static LIVE_RPC_CHAIN_STATUS: OnceLock<Arc<RwLock<BTreeMap<String, LiveRpcChainStatusRecord>>>> =
    OnceLock::new();

fn live_rpc_chain_status_store() -> &'static Arc<RwLock<BTreeMap<String, LiveRpcChainStatusRecord>>>
{
    LIVE_RPC_CHAIN_STATUS.get_or_init(|| Arc::new(RwLock::new(BTreeMap::new())))
}

pub fn live_rpc_chain_status_snapshot() -> Vec<LiveRpcChainStatus> {
    let now_unix_ms = current_unix_ms();
    let records = live_rpc_chain_status_store()
        .read()
        .map(|rows| rows.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    records
        .into_iter()
        .map(|record| {
            let silent_for_ms = record.last_pending_unix_ms.and_then(|last_seen| {
                let elapsed_ms = now_unix_ms.saturating_sub(last_seen);
                u64::try_from(elapsed_ms).ok()
            });
            LiveRpcChainStatus {
                chain_key: record.chain_key,
                chain_id: record.chain_id,
                source_id: record.source_id,
                state: record.state,
                endpoint_index: record.endpoint_index,
                endpoint_count: record.endpoint_count,
                ws_url: record.ws_url,
                http_url: record.http_url,
                last_pending_unix_ms: record.last_pending_unix_ms,
                silent_for_ms,
                updated_unix_ms: record.updated_unix_ms,
                last_error: record.last_error,
                rotation_count: record.rotation_count,
            }
        })
        .collect()
}

fn reset_live_rpc_chain_status() {
    if let Ok(mut rows) = live_rpc_chain_status_store().write() {
        rows.clear();
    }
}

fn update_live_rpc_chain_status(
    chain: &ChainRpcConfig,
    endpoint: &RpcEndpoint,
    endpoint_index: usize,
    state: &str,
    last_error: Option<String>,
    observed_pending: bool,
    increment_rotation: bool,
) {
    let now_unix_ms = current_unix_ms();
    let mut rows = match live_rpc_chain_status_store().write() {
        Ok(rows) => rows,
        Err(_) => return,
    };
    let entry = rows
        .entry(chain.chain_key.clone())
        .or_insert_with(|| LiveRpcChainStatusRecord {
            chain_key: chain.chain_key.clone(),
            chain_id: chain.chain_id,
            source_id: chain.source_id.to_string(),
            state: state.to_owned(),
            endpoint_index,
            endpoint_count: chain.endpoints.len(),
            ws_url: endpoint.ws_url.clone(),
            http_url: endpoint.http_url.clone(),
            last_pending_unix_ms: None,
            updated_unix_ms: now_unix_ms,
            last_error: None,
            rotation_count: 0,
        });
    entry.chain_id = chain.chain_id;
    entry.source_id = chain.source_id.to_string();
    entry.state = state.to_owned();
    entry.endpoint_index = endpoint_index;
    entry.endpoint_count = chain.endpoints.len();
    entry.ws_url = endpoint.ws_url.clone();
    entry.http_url = endpoint.http_url.clone();
    entry.updated_unix_ms = now_unix_ms;
    entry.last_error = last_error;
    if observed_pending || state == "subscribed" {
        entry.last_pending_unix_ms = Some(now_unix_ms);
    }
    if increment_rotation {
        entry.rotation_count = entry.rotation_count.saturating_add(1);
    }
}

#[derive(Clone, Debug)]
struct RpcEndpoint {
    ws_url: String,
    http_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct EnvRpcEndpointConfig {
    ws_url: String,
    http_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct EnvChainConfig {
    chain_key: String,
    #[serde(default)]
    chain_id: Option<u64>,
    #[serde(default)]
    ws_url: Option<String>,
    #[serde(default)]
    http_url: Option<String>,
    #[serde(default)]
    endpoints: Vec<EnvRpcEndpointConfig>,
    #[serde(default)]
    source_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ChainConfigInput {
    Wrapped { chains: Vec<EnvChainConfig> },
    List(Vec<EnvChainConfig>),
}

#[derive(Clone, Debug)]
pub struct ChainRpcConfig {
    chain_key: String,
    chain_id: Option<u64>,
    endpoints: Vec<RpcEndpoint>,
    source_id: SourceId,
}

impl ChainRpcConfig {
    pub fn chain_key(&self) -> &str {
        &self.chain_key
    }

    pub fn chain_id(&self) -> Option<u64> {
        self.chain_id
    }

    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    pub fn primary_ws_url(&self) -> Option<&str> {
        self.endpoints
            .first()
            .map(|endpoint| endpoint.ws_url.as_str())
    }

    pub fn primary_http_url(&self) -> Option<&str> {
        self.endpoints
            .first()
            .map(|endpoint| endpoint.http_url.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct LiveRpcConfig {
    chains: Vec<ChainRpcConfig>,
    max_seen_hashes: usize,
    batch_fetch: BatchFetchConfig,
    silent_chain_timeout_secs: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchFetchConfig {
    pub batch_size: usize,
    pub max_in_flight: usize,
    pub retry_attempts: usize,
    pub retry_backoff_ms: u64,
    pub flush_interval_ms: u64,
}

impl Default for BatchFetchConfig {
    fn default() -> Self {
        Self {
            batch_size: 32,
            max_in_flight: 4,
            retry_attempts: 2,
            retry_backoff_ms: 100,
            flush_interval_ms: 40,
        }
    }
}

impl Default for LiveRpcConfig {
    fn default() -> Self {
        Self {
            chains: vec![ChainRpcConfig {
                chain_key: "eth-mainnet".to_owned(),
                chain_id: Some(1),
                endpoints: vec![
                    RpcEndpoint {
                        ws_url: PRIMARY_PUBLIC_WS_URL.to_owned(),
                        http_url: PRIMARY_PUBLIC_HTTP_URL.to_owned(),
                    },
                    RpcEndpoint {
                        ws_url: FALLBACK_PUBLIC_WS_URL.to_owned(),
                        http_url: FALLBACK_PUBLIC_HTTP_URL.to_owned(),
                    },
                ],
                source_id: SourceId::new("rpc-live"),
            }],
            max_seen_hashes: 10_000,
            batch_fetch: BatchFetchConfig::default(),
            silent_chain_timeout_secs: DEFAULT_SILENT_CHAIN_TIMEOUT_SECS,
        }
    }
}

impl LiveRpcConfig {
    pub fn from_env() -> Result<Self> {
        let chains_override = read_env_trimmed(ENV_CHAINS);
        let ws_override = read_env_trimmed(ENV_ETH_WS_URL);
        let http_override = read_env_trimmed(ENV_ETH_HTTP_URL);
        let source_id_override = read_env_trimmed(ENV_SOURCE_ID);
        let max_seen_hashes = read_env_trimmed(ENV_MAX_SEEN_HASHES)
            .map(|value| value.parse::<usize>())
            .transpose()
            .map_err(|err| anyhow!("invalid {ENV_MAX_SEEN_HASHES}: {err}"))?
            .unwrap_or(10_000)
            .max(1);
        let batch_size = parse_env_usize(ENV_RPC_BATCH_SIZE)?
            .unwrap_or(BatchFetchConfig::default().batch_size)
            .max(1);
        let max_in_flight = parse_env_usize(ENV_RPC_MAX_IN_FLIGHT)?
            .unwrap_or(BatchFetchConfig::default().max_in_flight)
            .max(1);
        let retry_attempts = parse_env_usize(ENV_RPC_RETRY_ATTEMPTS)?
            .unwrap_or(BatchFetchConfig::default().retry_attempts);
        let retry_backoff_ms = parse_env_u64(ENV_RPC_RETRY_BACKOFF_MS)?
            .unwrap_or(BatchFetchConfig::default().retry_backoff_ms)
            .max(1);
        let flush_interval_ms = parse_env_u64(ENV_RPC_BATCH_FLUSH_MS)?
            .unwrap_or(BatchFetchConfig::default().flush_interval_ms)
            .max(1);
        let silent_chain_timeout_secs = parse_env_u64(ENV_SILENT_CHAIN_TIMEOUT_SECS)?
            .unwrap_or(DEFAULT_SILENT_CHAIN_TIMEOUT_SECS)
            .max(1);

        let mut config = Self::default();
        if let Some(file_chains) = load_chain_configs_from_file()? {
            config.chains = file_chains;
        }
        if let Some(chains_override) = chains_override {
            config.chains = parse_env_chain_configs(&chains_override)?;
        } else {
            if let Some(ws_url) = ws_override {
                if let Some(primary_chain) = config.chains.first_mut() {
                    primary_chain.endpoints = vec![RpcEndpoint {
                        ws_url,
                        http_url: http_override
                            .unwrap_or_else(|| PRIMARY_PUBLIC_HTTP_URL.to_owned()),
                    }];
                }
            } else if let Some(http_url) = http_override
                && let Some(primary_chain) = config.chains.first_mut()
                && let Some(primary_endpoint) = primary_chain.endpoints.first_mut()
            {
                primary_endpoint.http_url = http_url;
            }

            if let Some(source_id_override) = source_id_override
                && let Some(primary_chain) = config.chains.first_mut()
            {
                primary_chain.source_id = SourceId::new(source_id_override);
            }
        }
        config.max_seen_hashes = max_seen_hashes;
        config.batch_fetch = BatchFetchConfig {
            batch_size,
            max_in_flight,
            retry_attempts,
            retry_backoff_ms,
            flush_interval_ms,
        };
        config.silent_chain_timeout_secs = silent_chain_timeout_secs;
        Ok(config)
    }

    pub fn chain_configs(&self) -> &[ChainRpcConfig] {
        &self.chains
    }

    pub fn primary_ws_url(&self) -> Option<&str> {
        self.chains.first().and_then(|chain| chain.primary_ws_url())
    }

    pub fn primary_http_url(&self) -> Option<&str> {
        self.chains
            .first()
            .and_then(|chain| chain.primary_http_url())
    }

    pub fn source_id(&self) -> &SourceId {
        self.chains
            .first()
            .map(|chain| &chain.source_id)
            .unwrap_or_else(|| panic!("live rpc config has no chains"))
    }

    pub fn max_seen_hashes(&self) -> usize {
        self.max_seen_hashes
    }

    pub fn batch_fetch(&self) -> BatchFetchConfig {
        self.batch_fetch
    }

    pub fn silent_chain_timeout_secs(&self) -> u64 {
        self.silent_chain_timeout_secs
    }
}

pub fn worker_count_for_config(config: &LiveRpcConfig) -> usize {
    config.chains.len()
}

pub fn resolve_record_chain_id(
    configured_chain_id: Option<u64>,
    tx_chain_id: Option<u64>,
) -> Option<u64> {
    tx_chain_id.or(configured_chain_id)
}

pub fn coalesce_hash_batches(hashes: &[String], batch_size: usize) -> Vec<Vec<String>> {
    let batch_size = batch_size.max(1);
    hashes
        .chunks(batch_size)
        .map(|chunk| chunk.to_vec())
        .collect()
}

pub fn dispatchable_batch_count(
    queue_len: usize,
    batch_size: usize,
    max_in_flight: usize,
) -> usize {
    if queue_len == 0 {
        return 0;
    }
    let batches = queue_len.div_ceil(batch_size.max(1));
    batches.min(max_in_flight.max(1))
}

pub fn retry_backoff_delay_ms(base_backoff_ms: u64, attempt: usize) -> u64 {
    base_backoff_ms.saturating_mul(1_u64 << attempt.min(16))
}

pub fn rotate_endpoint_index(current: usize, endpoint_count: usize) -> usize {
    if endpoint_count <= 1 {
        0
    } else {
        (current + 1) % endpoint_count
    }
}

pub fn should_rotate_silent_chain(
    last_pending_unix_ms: i64,
    now_unix_ms: i64,
    timeout_seconds: u64,
) -> bool {
    if timeout_seconds == 0 {
        return false;
    }
    let timeout_ms = timeout_seconds.saturating_mul(1_000);
    let elapsed_ms = now_unix_ms.saturating_sub(last_pending_unix_ms);
    let elapsed_ms_u64 = u64::try_from(elapsed_ms).unwrap_or(0);
    elapsed_ms_u64 >= timeout_ms
}

fn read_env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn parse_env_usize(key: &str) -> Result<Option<usize>> {
    read_env_trimmed(key)
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|err| anyhow!("invalid {key}: {err}"))
}

fn parse_env_u64(key: &str) -> Result<Option<u64>> {
    read_env_trimmed(key)
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|err| anyhow!("invalid {key}: {err}"))
}

fn parse_env_chain_configs(raw: &str) -> Result<Vec<ChainRpcConfig>> {
    let env_chains = decode_chain_configs(raw, "VIZ_API_CHAINS")?;
    env_chains
        .into_iter()
        .map(parse_env_chain_config)
        .collect::<Result<Vec<_>>>()
}

fn load_chain_configs_from_file() -> Result<Option<Vec<ChainRpcConfig>>> {
    if let Some(path_override) = read_env_trimmed(ENV_CHAIN_CONFIG_PATH) {
        return Ok(Some(read_chain_configs_from_path(PathBuf::from(
            path_override,
        ))?));
    }

    for path in default_chain_config_paths() {
        match read_chain_configs_from_path(path.clone()) {
            Ok(chains) => return Ok(Some(chains)),
            Err(err) if is_file_not_found_error(&err) => continue,
            Err(err) => return Err(err),
        }
    }

    Ok(None)
}

fn default_chain_config_paths() -> [PathBuf; 2] {
    [
        PathBuf::from(DEFAULT_CHAIN_CONFIG_PATH),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(DEFAULT_CHAIN_CONFIG_PATH),
    ]
}

fn read_chain_configs_from_path(path: PathBuf) -> Result<Vec<ChainRpcConfig>> {
    let payload = fs::read_to_string(&path)
        .with_context(|| format!("read chain config file '{}'", path.display()))?;
    let source_label = format!("chain config file '{}'", path.display());
    let env_chains = decode_chain_configs(&payload, &source_label)?;
    env_chains
        .into_iter()
        .map(parse_env_chain_config)
        .collect::<Result<Vec<_>>>()
}

fn decode_chain_configs(raw: &str, source_label: &str) -> Result<Vec<EnvChainConfig>> {
    let env_chains = match serde_json::from_str::<ChainConfigInput>(raw)
        .with_context(|| format!("decode {source_label} json"))?
    {
        ChainConfigInput::Wrapped { chains } => chains,
        ChainConfigInput::List(chains) => chains,
    };

    if env_chains.is_empty() {
        return Err(anyhow!("{source_label} must contain at least one chain"));
    }
    Ok(env_chains)
}

fn is_file_not_found_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .map(|io_err| io_err.kind() == ErrorKind::NotFound)
        .unwrap_or(false)
}

fn parse_env_chain_config(chain: EnvChainConfig) -> Result<ChainRpcConfig> {
    let chain_key = chain.chain_key.trim();
    if chain_key.is_empty() {
        return Err(anyhow!("chain_key cannot be empty"));
    }
    let endpoints = parse_chain_endpoints(&chain, chain_key)?;

    let source_id = chain
        .source_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("rpc-{chain_key}"));

    Ok(ChainRpcConfig {
        chain_key: chain_key.to_owned(),
        chain_id: chain.chain_id,
        endpoints,
        source_id: SourceId::new(source_id),
    })
}

fn parse_chain_endpoints(chain: &EnvChainConfig, chain_key: &str) -> Result<Vec<RpcEndpoint>> {
    if !chain.endpoints.is_empty() {
        return chain
            .endpoints
            .iter()
            .map(|endpoint| parse_endpoint_urls(chain_key, &endpoint.ws_url, &endpoint.http_url))
            .collect();
    }

    let ws_url = chain.ws_url.as_deref().unwrap_or_default();
    let http_url = chain.http_url.as_deref().unwrap_or_default();
    let endpoint = parse_endpoint_urls(chain_key, ws_url, http_url)?;
    Ok(vec![endpoint])
}

fn parse_endpoint_urls(chain_key: &str, ws_url: &str, http_url: &str) -> Result<RpcEndpoint> {
    let ws_url = ws_url.trim();
    if ws_url.is_empty() {
        return Err(anyhow!("ws_url for chain '{chain_key}' cannot be empty"));
    }
    let http_url = http_url.trim();
    if http_url.is_empty() {
        return Err(anyhow!("http_url for chain '{chain_key}' cannot be empty"));
    }

    Ok(RpcEndpoint {
        ws_url: ws_url.to_owned(),
        http_url: http_url.to_owned(),
    })
}

pub fn start_live_rpc_feed(
    storage: Arc<RwLock<InMemoryStorage>>,
    writer: StorageWriteHandle,
    scheduler: SchedulerHandle,
    config: LiveRpcConfig,
) {
    start_live_rpc_feed_with_bootstrap(storage, writer, scheduler, config, false);
}

pub fn start_live_rpc_feed_with_bootstrap(
    storage: Arc<RwLock<InMemoryStorage>>,
    writer: StorageWriteHandle,
    scheduler: SchedulerHandle,
    config: LiveRpcConfig,
    bootstrap_from_pending_pool: bool,
) {
    LIVE_RPC_FEED_START_COUNT.fetch_add(1, Ordering::Relaxed);
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => return,
    };

    if config.chains.is_empty() {
        tracing::error!("live rpc config has no chains configured");
        return;
    }

    let next_seq_id = Arc::new(AtomicU64::new(
        current_seq_hi(&storage).saturating_add(1).max(1),
    ));
    reset_live_rpc_drop_metrics();
    reset_live_rpc_chain_status();
    let max_seen_hashes = config.max_seen_hashes;
    let batch_fetch = config.batch_fetch;
    let silent_chain_timeout_secs = config.silent_chain_timeout_secs;

    for chain in config.chains {
        let writer = writer.clone();
        let next_seq_id = next_seq_id.clone();
        let scheduler = scheduler.clone();
        let bootstrap_from_pending_pool = bootstrap_from_pending_pool;
        handle.spawn(async move {
            run_chain_worker(
                writer,
                scheduler,
                chain,
                bootstrap_from_pending_pool,
                max_seen_hashes,
                batch_fetch,
                silent_chain_timeout_secs,
                next_seq_id,
            )
            .await;
        });
    }
}

pub fn start_live_rpc_pending_pool_rebuild(
    storage: Arc<RwLock<InMemoryStorage>>,
    writer: StorageWriteHandle,
    scheduler: SchedulerHandle,
    config: LiveRpcConfig,
) {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => return,
    };

    if config.chains.is_empty() {
        tracing::error!("live rpc config has no chains configured");
        return;
    }

    let next_seq_id = Arc::new(AtomicU64::new(
        current_seq_hi(&storage).saturating_add(1).max(1),
    ));
    for chain in config.chains {
        let writer = writer.clone();
        let next_seq_id = next_seq_id.clone();
        let scheduler = scheduler.clone();
        handle.spawn(async move {
            run_chain_pending_pool_rebuild(writer, scheduler, chain, next_seq_id).await;
        });
    }
}

async fn run_chain_worker(
    writer: StorageWriteHandle,
    scheduler: SchedulerHandle,
    chain: ChainRpcConfig,
    bootstrap_from_pending_pool: bool,
    max_seen_hashes: usize,
    batch_fetch: BatchFetchConfig,
    silent_chain_timeout_secs: u64,
    next_seq_id: Arc<AtomicU64>,
) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            tracing::error!(
                error = %err,
                chain_key = %chain.chain_key,
                "failed to build reqwest client for chain ingest worker"
            );
            return;
        }
    };
    if chain.endpoints.is_empty() {
        tracing::error!(
            chain_key = %chain.chain_key,
            "live rpc chain config has no endpoints"
        );
        return;
    }

    if bootstrap_from_pending_pool {
        match rebuild_scheduler_from_pending_pool(
            &writer,
            &scheduler,
            &chain,
            &client,
            &next_seq_id,
        )
        .await
        {
            Ok(rebuilt) => {
                tracing::info!(
                    chain_key = %chain.chain_key,
                    chain_id = ?chain.chain_id,
                    rebuilt,
                    "rebuilt scheduler state from rpc pending pool before websocket ingest"
                );
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    chain_key = %chain.chain_key,
                    chain_id = ?chain.chain_id,
                    "failed to rebuild scheduler state from rpc pending pool before websocket ingest"
                );
            }
        }
    }

    let mut seen_hashes = FastSet::default();
    let mut seen_order = VecDeque::new();
    let mut endpoint_index = 0usize;
    if let Some(initial_endpoint) = chain.endpoints.first() {
        update_live_rpc_chain_status(
            &chain,
            initial_endpoint,
            endpoint_index,
            "booting",
            None,
            false,
            false,
        );
    }

    loop {
        let endpoint = &chain.endpoints[endpoint_index];
        update_live_rpc_chain_status(
            &chain,
            endpoint,
            endpoint_index,
            "connecting",
            None,
            false,
            false,
        );
        tracing::info!(
            chain_key = %chain.chain_key,
            chain_id = ?chain.chain_id,
            ws_url = endpoint.ws_url,
            http_url = endpoint.http_url,
            endpoint_index,
            "starting chain-scoped live rpc websocket session"
        );
        let session_result = run_ws_session(
            LiveRpcSessionContext {
                writer: &writer,
                scheduler: &scheduler,
                chain: &chain,
                endpoint,
                endpoint_index,
                max_seen_hashes,
                batch_fetch,
                silent_chain_timeout_secs,
                client: &client,
                next_seq_id: &next_seq_id,
            },
            &mut seen_hashes,
            &mut seen_order,
        )
        .await;

        if let Err(err) = session_result {
            let error_chain = format_error_chain(&err);
            let state = if error_chain.contains("silent_chain_timeout") {
                "silent_timeout"
            } else {
                "error"
            };
            update_live_rpc_chain_status(
                &chain,
                endpoint,
                endpoint_index,
                state,
                Some(error_chain.clone()),
                false,
                false,
            );
            tracing::warn!(
                error = %err,
                error_chain = %error_chain,
                chain_key = %chain.chain_key,
                chain_id = ?chain.chain_id,
                ws_url = endpoint.ws_url,
                http_url = endpoint.http_url,
                "live rpc websocket session ended for chain; rotating endpoint"
            );
            endpoint_index = rotate_endpoint_index(endpoint_index, chain.endpoints.len());
            let next_endpoint = &chain.endpoints[endpoint_index];
            update_live_rpc_chain_status(
                &chain,
                next_endpoint,
                endpoint_index,
                "rotating",
                Some(error_chain),
                false,
                true,
            );
        } else {
            update_live_rpc_chain_status(
                &chain,
                endpoint,
                endpoint_index,
                "disconnected",
                None,
                false,
                false,
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn run_chain_pending_pool_rebuild(
    writer: StorageWriteHandle,
    scheduler: SchedulerHandle,
    chain: ChainRpcConfig,
    next_seq_id: Arc<AtomicU64>,
) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            tracing::error!(
                error = %err,
                chain_key = %chain.chain_key,
                "failed to build reqwest client for pending-pool rebuild"
            );
            return;
        }
    };
    if chain.endpoints.is_empty() {
        tracing::error!(
            chain_key = %chain.chain_key,
            "live rpc chain config has no endpoints"
        );
        return;
    }

    match rebuild_scheduler_from_pending_pool(&writer, &scheduler, &chain, &client, &next_seq_id)
        .await
    {
        Ok(rebuilt) => {
            tracing::info!(
                chain_key = %chain.chain_key,
                chain_id = ?chain.chain_id,
                rebuilt,
                "rebuilt scheduler state from rpc pending pool"
            );
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                chain_key = %chain.chain_key,
                chain_id = ?chain.chain_id,
                "failed to rebuild scheduler state from rpc pending pool"
            );
        }
    }
}

struct LiveRpcSessionContext<'a> {
    writer: &'a StorageWriteHandle,
    scheduler: &'a SchedulerHandle,
    chain: &'a ChainRpcConfig,
    endpoint: &'a RpcEndpoint,
    endpoint_index: usize,
    max_seen_hashes: usize,
    batch_fetch: BatchFetchConfig,
    silent_chain_timeout_secs: u64,
    client: &'a reqwest::Client,
    next_seq_id: &'a Arc<AtomicU64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingHashObservation {
    hash_hex: String,
    observed_at_unix_ms: i64,
    observed_at_mono_ns: u64,
}

async fn run_ws_session(
    session: LiveRpcSessionContext<'_>,
    seen_hashes: &mut FastSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(session.endpoint.ws_url.as_str())
        .await
        .with_context(|| format!("connect {}", session.endpoint.ws_url))?;
    let (mut write, mut read) = ws_stream.split();

    let subscribe = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["newPendingTransactions"],
    });
    write
        .send(Message::Text(subscribe.to_string().into()))
        .await
        .context("send eth_subscribe request")?;
    tracing::info!(
        chain_key = %session.chain.chain_key,
        chain_id = ?session.chain.chain_id,
        ws_url = session.endpoint.ws_url,
        "subscribed to eth_subscribe:newPendingTransactions"
    );
    update_live_rpc_chain_status(
        session.chain,
        session.endpoint,
        session.endpoint_index,
        "subscribed",
        None,
        false,
        false,
    );

    let mut subscription_id: Option<String> = None;
    let flush_interval = Duration::from_millis(session.batch_fetch.flush_interval_ms);
    let mut pending_hashes: VecDeque<PendingHashObservation> = VecDeque::new();
    let in_flight = Arc::new(Semaphore::new(session.batch_fetch.max_in_flight));
    let mut last_pending_unix_ms = current_unix_ms();
    loop {
        match tokio::time::timeout(flush_interval, read.next()).await {
            Ok(Some(frame)) => {
                let frame = frame.context("read websocket frame")?;
                match frame {
                    Message::Text(text) => {
                        if let Some(hash_hex) = parse_pending_hash(&text, &mut subscription_id) {
                            let observed_at_unix_ms = current_unix_ms();
                            pending_hashes.push_back(PendingHashObservation {
                                hash_hex: hash_hex.to_owned(),
                                observed_at_unix_ms,
                                observed_at_mono_ns: current_mono_ns(),
                            });
                            last_pending_unix_ms = observed_at_unix_ms;
                            update_live_rpc_chain_status(
                                session.chain,
                                session.endpoint,
                                session.endpoint_index,
                                "active",
                                None,
                                true,
                                false,
                            );
                        }
                    }
                    Message::Ping(payload) => {
                        if write.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
                flush_pending_hash_batches(
                    &session,
                    seen_hashes,
                    seen_order,
                    &mut pending_hashes,
                    &in_flight,
                )
                .await?;
            }
            Ok(None) => break,
            Err(_) => {
                flush_pending_hash_batches(
                    &session,
                    seen_hashes,
                    seen_order,
                    &mut pending_hashes,
                    &in_flight,
                )
                .await?;
            }
        }
        if should_rotate_silent_chain(
            last_pending_unix_ms,
            current_unix_ms(),
            session.silent_chain_timeout_secs,
        ) {
            return Err(anyhow!(
                "silent_chain_timeout: no pending notifications for {}s",
                session.silent_chain_timeout_secs
            ));
        }
    }
    flush_pending_hash_batches(
        &session,
        seen_hashes,
        seen_order,
        &mut pending_hashes,
        &in_flight,
    )
    .await?;

    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WsPendingHashValue<'a> {
    Str(&'a str),
    Obj(WsPendingHashObject<'a>),
}

impl<'a> WsPendingHashValue<'a> {
    fn as_hash(&self) -> Option<&'a str> {
        match self {
            Self::Str(hash) => Some(*hash),
            Self::Obj(obj) => obj.hash,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WsPendingHashObject<'a> {
    #[serde(default, borrow)]
    hash: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct WsSubscriptionParams<'a> {
    #[serde(borrow)]
    subscription: &'a str,
    #[serde(borrow)]
    result: WsPendingHashValue<'a>,
}

#[derive(Debug, Deserialize)]
struct WsRpcMessage<'a> {
    #[serde(default)]
    id: Option<IgnoredAny>,
    #[serde(default, borrow)]
    method: Option<&'a str>,
    #[serde(default, borrow)]
    result: Option<WsPendingHashValue<'a>>,
    #[serde(default, borrow)]
    params: Option<WsSubscriptionParams<'a>>,
}

pub fn parse_pending_hash<'a>(
    payload: &'a str,
    subscription_id: &mut Option<String>,
) -> Option<&'a str> {
    let message: WsRpcMessage<'a> = serde_json::from_str(payload).ok()?;
    if message.id.is_some() {
        if let Some(result) = message
            .result
            .as_ref()
            .and_then(WsPendingHashValue::as_hash)
        {
            *subscription_id = Some(result.to_owned());
        }
        return None;
    }
    if message.method != Some("eth_subscription") {
        return None;
    }

    let params = message.params?;
    if let Some(expected) = subscription_id.as_ref()
        && expected != params.subscription
    {
        return None;
    }

    params.result.as_hash()
}

async fn flush_pending_hash_batches(
    session: &LiveRpcSessionContext<'_>,
    seen_hashes: &mut FastSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
    pending_hashes: &mut VecDeque<PendingHashObservation>,
    in_flight: &Arc<Semaphore>,
) -> Result<()> {
    let dispatch_count = dispatchable_batch_count(
        pending_hashes.len(),
        session.batch_fetch.batch_size,
        session.batch_fetch.max_in_flight,
    );
    for _ in 0..dispatch_count {
        let batch = pending_hashes
            .drain(..session.batch_fetch.batch_size.min(pending_hashes.len()))
            .collect::<Vec<_>>();
        process_pending_hash_batch(session, seen_hashes, seen_order, batch, in_flight).await?;
    }
    Ok(())
}

async fn process_pending_hash_batch(
    session: &LiveRpcSessionContext<'_>,
    seen_hashes: &mut FastSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
    hash_batch: Vec<PendingHashObservation>,
    in_flight: &Arc<Semaphore>,
) -> Result<()> {
    let mut deduped_observations = Vec::new();
    let mut deduped_raw = Vec::new();
    for observation in hash_batch {
        let hash = match parse_fixed_hex::<32>(&observation.hash_hex) {
            Some(hash) => hash,
            None => {
                observe_live_rpc_drop_reason(LiveRpcDropReason::InvalidPendingHash);
                tracing::warn!(
                    chain_key = %session.chain.chain_key,
                    source_id = %session.chain.source_id,
                    hash = %observation.hash_hex,
                    reason = LiveRpcDropReason::InvalidPendingHash.as_label(),
                    "dropping pending hash: invalid hex payload"
                );
                continue;
            }
        };
        if remember_hash(hash, seen_hashes, seen_order, session.max_seen_hashes) {
            deduped_observations.push(observation);
            deduped_raw.push(hash);
        }
    }
    if deduped_observations.is_empty() {
        return Ok(());
    }

    let _permit = in_flight
        .acquire()
        .await
        .map_err(|_| anyhow!("rpc in-flight semaphore closed"))?;
    let rpc_started = Instant::now();
    let fetched = fetch_transactions_by_hash_batch_with_retry(
        session.client,
        session.endpoint.http_url.as_str(),
        &deduped_observations
            .iter()
            .map(|observation| observation.hash_hex.clone())
            .collect::<Vec<_>>(),
        session.batch_fetch,
    )
    .await?;
    let rpc_latency_ms = rpc_started.elapsed().as_secs_f64() * 1_000.0;
    let fetched_count = fetched.iter().filter(|row| row.is_some()).count();
    let error_count = deduped_observations.len().saturating_sub(fetched_count);
    let error_rate = error_count as f64 / deduped_observations.len() as f64;
    tracing::info!(
        chain_key = %session.chain.chain_key,
        chain_id = ?session.chain.chain_id,
        batch_size = deduped_observations.len(),
        fetched_count,
        error_count,
        error_rate,
        rpc_latency_ms,
        "live rpc batch fetch metrics"
    );

    for ((observation, hash), fetched_tx) in deduped_observations
        .iter()
        .zip(deduped_raw.into_iter())
        .zip(fetched.into_iter())
    {
        process_pending_hash_with_fetched_tx(
            session.writer,
            session.scheduler,
            session.chain,
            observation,
            hash,
            fetched_tx,
            session.next_seq_id,
        )
        .await?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchedulerPersistenceDecision {
    Admitted,
    Duplicate,
    Replaced { replaced_hash: TxHash },
    Dropped { reason: &'static str },
}

impl SchedulerPersistenceDecision {
    fn from_admission(admission: SchedulerAdmission) -> Self {
        match admission {
            SchedulerAdmission::Admitted => Self::Admitted,
            SchedulerAdmission::Duplicate => Self::Duplicate,
            SchedulerAdmission::Replaced { replaced_hash } => Self::Replaced { replaced_hash },
            SchedulerAdmission::UnderpricedReplacement => Self::Dropped {
                reason: "underpriced_replacement",
            },
            SchedulerAdmission::SenderLimitReached => Self::Dropped {
                reason: "sender_limit_reached",
            },
        }
    }

    fn is_admitted(self) -> bool {
        matches!(self, Self::Admitted | Self::Replaced { .. })
    }

    fn emits_decoded(self) -> bool {
        self.is_admitted()
    }

    fn replaced_hash(self) -> Option<TxHash> {
        match self {
            Self::Replaced { replaced_hash } => Some(replaced_hash),
            _ => None,
        }
    }

    fn dropped_reason(self) -> Option<&'static str> {
        match self {
            Self::Dropped { reason } => Some(reason),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Duplicate => "duplicate",
            Self::Replaced { .. } => "replaced",
            Self::Dropped { reason } => reason,
        }
    }
}

async fn process_pending_hash_with_fetched_tx(
    writer: &StorageWriteHandle,
    scheduler: &SchedulerHandle,
    chain: &ChainRpcConfig,
    observation: &PendingHashObservation,
    hash: TxHash,
    fetched_tx: Option<LiveTx>,
    next_seq_id: &Arc<AtomicU64>,
) -> Result<()> {
    let processed_at_unix_ms = current_unix_ms();
    let feature_analysis = fetched_tx
        .as_ref()
        .map(|tx| analyze_transaction(feature_input(tx)));
    let resolved_chain_id = fetched_tx
        .as_ref()
        .map(|tx| resolve_record_chain_id(chain.chain_id, tx.chain_id))
        .unwrap_or(chain.chain_id);

    if let Some(tx) = fetched_tx.as_ref() {
        let to = format_optional_fixed_hex(tx.to.as_ref().map(|value| value.as_slice()));
        let chain_id = format_optional_u64(resolved_chain_id);
        let gas_limit = format_optional_u64(tx.gas_limit);
        let value_wei = format_optional_u128(tx.value_wei);
        let gas_price_wei = format_optional_u128(tx.gas_price_wei);
        let max_fee_per_gas_wei = format_optional_u128(tx.max_fee_per_gas_wei);
        let max_priority_fee_per_gas_wei = format_optional_u128(tx.max_priority_fee_per_gas_wei);
        let max_fee_per_blob_gas_wei = format_optional_u128(tx.max_fee_per_blob_gas_wei);
        let analysis = feature_analysis.unwrap_or(default_feature_analysis());
        let method_selector = format_method_selector_hex(analysis.method_selector);
        tracing::info!(
            hash = %observation.hash_hex,
            chain_key = %chain.chain_key,
            source_id = %chain.source_id,
            sender = %format_fixed_hex(&tx.sender),
            to = %to,
            nonce = tx.nonce,
            tx_type = tx.tx_type,
            chain_id = %chain_id,
            gas_limit = %gas_limit,
            value_wei = %value_wei,
            gas_price_wei = %gas_price_wei,
            max_fee_per_gas_wei = %max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei = %max_priority_fee_per_gas_wei,
            max_fee_per_blob_gas_wei = %max_fee_per_blob_gas_wei,
            protocol = analysis.protocol,
            category = analysis.category,
            mev_score = analysis.mev_score,
            urgency_score = analysis.urgency_score,
            method_selector = %method_selector,
            input_bytes = tx.input.len(),
            "mempool transaction"
        );
    } else {
        tracing::info!(
            hash = %observation.hash_hex,
            chain_key = %chain.chain_key,
            source_id = %chain.source_id,
            "mempool transaction (details unavailable from rpc)"
        );
    }

    let scheduler_result = if let Some(tx) = fetched_tx.as_ref() {
        let validated = validated_transaction_from_live_tx(
            chain,
            observation.observed_at_unix_ms,
            observation.observed_at_mono_ns,
            tx,
        );
        let (decision, queue_transitions) = match scheduler.admit_outcome(validated).await {
            Ok(outcome) => (
                SchedulerPersistenceDecision::from_admission(outcome.admission),
                outcome.queue_transitions,
            ),
            Err(SchedulerEnqueueError::QueueFull) => (
                SchedulerPersistenceDecision::Dropped {
                    reason: "queue_full",
                },
                Vec::new(),
            ),
            Err(SchedulerEnqueueError::QueueClosed) => {
                return Err(anyhow!("scheduler admission failed: queue closed"));
            }
        };
        tracing::debug!(
            chain_key = %chain.chain_key,
            source_id = %chain.source_id,
            hash = %observation.hash_hex,
            admission = decision.label(),
            admitted = decision.is_admitted(),
            "scheduler admission applied before legacy persistence"
        );
        Some((decision, queue_transitions))
    } else {
        None
    };

    let _tx_seen_seq_id = next_seq_id.fetch_add(1, Ordering::Relaxed);
    if !try_enqueue_storage_write(
        writer,
        chain,
        StorageWriteOp::AppendPayload {
            source_id: chain.source_id.clone(),
            ingest_ts_unix_ms: observation.observed_at_unix_ms,
            ingest_ts_mono_ns: observation.observed_at_mono_ns,
            payload: EventPayload::TxSeen(TxSeen {
                hash,
                peer_id: "rpc-ws".to_owned(),
                seen_at_unix_ms: observation.observed_at_unix_ms,
                seen_at_mono_ns: observation.observed_at_mono_ns,
            }),
        },
    )? {
        return Ok(());
    }
    if !try_enqueue_storage_write(
        writer,
        chain,
        StorageWriteOp::UpsertTxSeen(TxSeenRecord {
            hash,
            peer: "rpc-ws".to_owned(),
            first_seen_unix_ms: observation.observed_at_unix_ms,
            first_seen_mono_ns: observation.observed_at_mono_ns,
            seen_count: 1,
        }),
    )? {
        return Ok(());
    }

    if !append_event(
        writer,
        chain,
        next_seq_id,
        processed_at_unix_ms,
        EventPayload::TxFetched(TxFetched {
            hash,
            fetched_at_unix_ms: processed_at_unix_ms,
        }),
    )? {
        return Ok(());
    }

    if let Some(tx) = fetched_tx {
        let analysis = feature_analysis.unwrap_or(default_feature_analysis());
        let raw_tx = tx.input.clone();
        let validated = validated_transaction_from_live_tx(
            chain,
            observation.observed_at_unix_ms,
            observation.observed_at_mono_ns,
            &tx,
        );
        let decoded = validated.decoded.clone();
        let (scheduler_decision, queue_transitions) =
            scheduler_result.expect("decision exists when tx exists");

        if !try_enqueue_storage_write(
            writer,
            chain,
            StorageWriteOp::UpsertTxFull(TxFullRecord {
                hash: tx.hash,
                tx_type: tx.tx_type,
                sender: tx.sender,
                nonce: tx.nonce,
                to: tx.to,
                chain_id: resolved_chain_id,
                value_wei: tx.value_wei,
                gas_limit: tx.gas_limit,
                gas_price_wei: tx.gas_price_wei,
                max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
                max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
                max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
                calldata_len: Some(raw_tx.len() as u32),
                raw_tx: raw_tx.clone(),
            }),
        )? {
            return Ok(());
        }
        if !try_enqueue_storage_write(
            writer,
            chain,
            StorageWriteOp::UpsertTxFeatures(TxFeaturesRecord {
                hash: tx.hash,
                protocol: analysis.protocol.to_owned(),
                category: analysis.category.to_owned(),
                chain_id: resolved_chain_id,
                mev_score: analysis.mev_score,
                urgency_score: analysis.urgency_score,
                method_selector: analysis.method_selector,
                feature_engine_version: feature_engine_version().to_owned(),
            }),
        )? {
            return Ok(());
        }
        if let Some(replaced_hash) = scheduler_decision.replaced_hash() {
            if !append_event(
                writer,
                chain,
                next_seq_id,
                processed_at_unix_ms,
                EventPayload::TxReplaced(TxReplaced {
                    hash: replaced_hash,
                    replaced_by: tx.hash,
                }),
            )? {
                return Ok(());
            }
            live_rpc_simulation_service().invalidate_hash(replaced_hash);
        }
        if scheduler_decision.emits_decoded()
            && !append_event(
                writer,
                chain,
                next_seq_id,
                processed_at_unix_ms,
                EventPayload::TxDecoded(decoded.clone()),
            )?
        {
            return Ok(());
        }
        if let Some(reason) = scheduler_decision.dropped_reason() {
            if !append_event(
                writer,
                chain,
                next_seq_id,
                processed_at_unix_ms,
                EventPayload::TxDropped(TxDropped {
                    hash: tx.hash,
                    reason: reason.to_owned(),
                }),
            )? {
                return Ok(());
            }
        }
        let executable_transactions =
            ready_transactions_for_queue_transitions(scheduler, &queue_transitions);
        let simulation_context_transactions =
            ready_frontier_for_queue_transitions(scheduler, &queue_transitions);
        for transition in queue_transitions {
            if !append_queue_transition_event(
                writer,
                chain,
                next_seq_id,
                processed_at_unix_ms,
                transition,
            )? {
                return Ok(());
            }
        }
        let executable_opportunities = build_executable_opportunities(
            &executable_transactions,
            processed_at_unix_ms,
            resolved_chain_id,
        );
        if !executable_transactions.is_empty() {
            let executable_records = executable_opportunities
                .iter()
                .map(|opportunity| opportunity.record.clone())
                .collect::<Vec<_>>();
            let legacy_shadow_opportunities = build_legacy_comparison_opportunity_records(
                &executable_transactions,
                processed_at_unix_ms,
                resolved_chain_id,
            );
            if !executable_records.is_empty() || !legacy_shadow_opportunities.is_empty() {
                live_rpc_searcher_metrics()
                    .observe_batch(&executable_records, &legacy_shadow_opportunities);
            }
        }
        for opportunity in &executable_opportunities {
            if !append_event(
                writer,
                chain,
                next_seq_id,
                processed_at_unix_ms,
                build_opportunity_detected_payload(&opportunity.record),
            )? {
                return Ok(());
            }
        }
        for (task_key, request, opportunities) in
            build_simulation_batches(&executable_opportunities, &simulation_context_transactions)
        {
            live_rpc_simulation_service().enqueue(SimulationTask {
                task_key,
                generation: 0,
                chain: chain.clone(),
                request,
                opportunities,
                writer: writer.clone(),
                next_seq_id: next_seq_id.clone(),
                enqueued_unix_ms: processed_at_unix_ms,
            })?;
        }
    }

    Ok(())
}

async fn rebuild_scheduler_from_pending_pool(
    writer: &StorageWriteHandle,
    scheduler: &SchedulerHandle,
    chain: &ChainRpcConfig,
    client: &reqwest::Client,
    next_seq_id: &Arc<AtomicU64>,
) -> Result<usize> {
    let mut last_error: Option<anyhow::Error> = None;
    for endpoint in &chain.endpoints {
        match fetch_pending_block_transactions(client, endpoint.http_url.as_str()).await {
            Ok(pending) => {
                return rebuild_scheduler_from_pending_transactions(
                    writer,
                    scheduler,
                    chain,
                    &pending,
                    next_seq_id,
                )
                .await;
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    chain_key = %chain.chain_key,
                    chain_id = ?chain.chain_id,
                    http_url = endpoint.http_url,
                    "rpc pending-pool rebuild fetch failed; trying next endpoint"
                );
                last_error = Some(err);
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow!("no rpc endpoints configured for pending-pool rebuild")))
}

async fn rebuild_scheduler_from_pending_transactions(
    writer: &StorageWriteHandle,
    scheduler: &SchedulerHandle,
    chain: &ChainRpcConfig,
    pending: &[LiveTx],
    next_seq_id: &Arc<AtomicU64>,
) -> Result<usize> {
    let mut rebuilt = 0usize;
    let mut seen_hashes = FastSet::default();

    for tx in pending {
        if !seen_hashes.insert(tx.hash) {
            continue;
        }

        let observation = PendingHashObservation {
            hash_hex: format_fixed_hex(&tx.hash),
            observed_at_unix_ms: current_unix_ms(),
            observed_at_mono_ns: current_mono_ns(),
        };
        process_pending_hash_with_fetched_tx(
            writer,
            scheduler,
            chain,
            &observation,
            tx.hash,
            Some(tx.clone()),
            next_seq_id,
        )
        .await?;
        rebuilt = rebuilt.saturating_add(1);
    }

    Ok(rebuilt)
}

fn append_queue_transition_event(
    writer: &StorageWriteHandle,
    chain: &ChainRpcConfig,
    next_seq_id: &Arc<AtomicU64>,
    now_unix_ms: i64,
    transition: SchedulerQueueTransition,
) -> Result<bool> {
    let payload = match transition.state {
        SchedulerQueueState::Ready => EventPayload::TxReady(TxReady {
            hash: transition.hash,
            sender: transition.sender,
            nonce: transition.nonce,
        }),
        SchedulerQueueState::Blocked { expected_nonce } => EventPayload::TxBlocked(TxBlocked {
            hash: transition.hash,
            sender: transition.sender,
            nonce: transition.nonce,
            expected_nonce: Some(expected_nonce),
        }),
    };
    append_event(writer, chain, next_seq_id, now_unix_ms, payload)
}

fn validated_transaction_from_live_tx(
    chain: &ChainRpcConfig,
    observed_at_unix_ms: i64,
    observed_at_mono_ns: u64,
    tx: &LiveTx,
) -> ValidatedTransaction {
    ValidatedTransaction {
        source_id: chain.source_id.clone(),
        observed_at_unix_ms,
        observed_at_mono_ns,
        calldata: tx.input.clone(),
        decoded: TxDecoded {
            hash: tx.hash,
            tx_type: tx.tx_type,
            sender: tx.sender,
            nonce: tx.nonce,
            chain_id: tx.chain_id.or(chain.chain_id),
            to: tx.to,
            value_wei: tx.value_wei,
            gas_limit: tx.gas_limit,
            gas_price_wei: tx.gas_price_wei,
            max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
            max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
            calldata_len: Some(tx.input.len() as u32),
        },
    }
}

fn append_event(
    writer: &StorageWriteHandle,
    chain: &ChainRpcConfig,
    next_seq_id: &Arc<AtomicU64>,
    now_unix_ms: i64,
    payload: EventPayload,
) -> Result<bool> {
    let seq_id = next_seq_id.fetch_add(1, Ordering::Relaxed);
    try_enqueue_storage_write(
        writer,
        chain,
        StorageWriteOp::AppendPayload {
            source_id: chain.source_id.clone(),
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: seq_id.saturating_mul(1_000_000),
            payload,
        },
    )
}

fn try_enqueue_storage_write(
    writer: &StorageWriteHandle,
    chain: &ChainRpcConfig,
    op: StorageWriteOp,
) -> Result<bool> {
    match writer.try_enqueue(op) {
        Ok(()) => Ok(true),
        Err(error) => {
            let reason = classify_storage_enqueue_drop_reason(error);
            observe_live_rpc_drop_reason(reason);
            tracing::warn!(
                chain_key = %chain.chain_key,
                source_id = %chain.source_id,
                reason = reason.as_label(),
                "storage writer backpressure/drop observed for live rpc ingest"
            );
            match error {
                StorageTryEnqueueError::QueueFull => Ok(false),
                StorageTryEnqueueError::QueueClosed => {
                    Err(anyhow!("storage writer queue is closed"))
                }
            }
        }
    }
}

fn remember_hash(
    hash: TxHash,
    seen_hashes: &mut FastSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
    max_seen_hashes: usize,
) -> bool {
    if !seen_hashes.insert(hash) {
        return false;
    }
    seen_order.push_back(hash);
    while seen_order.len() > max_seen_hashes {
        if let Some(old_hash) = seen_order.pop_front() {
            seen_hashes.remove(&old_hash);
        }
    }
    true
}

#[derive(Debug, Deserialize)]
struct RpcTransaction {
    hash: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    gas: Option<String>,
    #[serde(default, rename = "type")]
    tx_type: Option<String>,
    #[serde(default, rename = "chainId")]
    chain_id: Option<String>,
    #[serde(default, rename = "gasPrice")]
    gas_price: Option<String>,
    #[serde(default, rename = "maxFeePerGas")]
    max_fee_per_gas: Option<String>,
    #[serde(default, rename = "maxPriorityFeePerGas")]
    max_priority_fee_per_gas: Option<String>,
    #[serde(default, rename = "maxFeePerBlobGas")]
    max_fee_per_blob_gas: Option<String>,
    #[serde(default)]
    input: Option<String>,
}

#[derive(Clone, Debug)]
struct LiveTx {
    hash: TxHash,
    sender: [u8; 20],
    to: Option<[u8; 20]>,
    nonce: u64,
    tx_type: u8,
    value_wei: Option<u128>,
    gas_limit: Option<u64>,
    chain_id: Option<u64>,
    gas_price_wei: Option<u128>,
    max_fee_per_gas_wei: Option<u128>,
    max_priority_fee_per_gas_wei: Option<u128>,
    max_fee_per_blob_gas_wei: Option<u128>,
    input: Vec<u8>,
}

#[inline]
fn feature_input(tx: &LiveTx) -> FeatureInput<'_> {
    FeatureInput {
        to: tx.to.as_ref(),
        calldata: &tx.input,
        tx_type: tx.tx_type,
        chain_id: tx.chain_id,
        gas_limit: tx.gas_limit,
        value_wei: tx.value_wei,
        gas_price_wei: tx.gas_price_wei,
        max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
    }
}

#[inline]
fn default_feature_analysis() -> FeatureAnalysis {
    FeatureAnalysis {
        protocol: "unknown",
        category: "pending",
        mev_score: 0,
        urgency_score: 0,
        method_selector: None,
    }
}

fn build_opportunity_records(
    decoded: &TxDecoded,
    calldata: &[u8],
    detected_unix_ms: i64,
    chain_id: Option<u64>,
) -> Vec<OpportunityRecord> {
    rank_opportunity_batch(
        &[SearcherInputTx {
            decoded: decoded.clone(),
            calldata: calldata.to_vec().into(),
        }],
        SearcherConfig {
            min_score: SEARCHER_MIN_SCORE,
            max_candidates: SEARCHER_MAX_CANDIDATES,
        },
    )
    .candidates
    .into_iter()
    .map(|candidate| OpportunityRecord {
        tx_hash: candidate.tx_hash,
        strategy: format!("{:?}", candidate.strategy),
        score: candidate.score,
        protocol: candidate.protocol,
        category: candidate.category,
        feature_engine_version: candidate.feature_engine_version,
        scorer_version: candidate.scorer_version,
        strategy_version: candidate.strategy_version,
        reasons: candidate.reasons,
        detected_unix_ms,
        chain_id,
    })
    .collect()
}

fn build_legacy_comparison_opportunity_records(
    executable: &[ValidatedTransaction],
    detected_unix_ms: i64,
    chain_id: Option<u64>,
) -> Vec<OpportunityRecord> {
    let mut candidates = executable
        .iter()
        .flat_map(|tx| {
            build_opportunity_records(
                &tx.decoded,
                tx.calldata.as_slice(),
                detected_unix_ms,
                chain_id,
            )
        })
        .collect::<Vec<_>>();
    sort_and_limit_opportunity_records(&mut candidates);
    candidates
}

fn build_executable_opportunities(
    executable: &[ValidatedTransaction],
    detected_unix_ms: i64,
    chain_id: Option<u64>,
) -> Vec<ExecutableOpportunity> {
    if executable.is_empty() {
        return Vec::new();
    }

    let batch = executable
        .iter()
        .map(|tx| SearcherInputTx::borrowed(tx.decoded.clone(), tx.calldata.as_slice()))
        .collect::<Vec<_>>();

    rank_opportunity_batch(
        &batch,
        SearcherConfig {
            min_score: SEARCHER_MIN_SCORE,
            max_candidates: SEARCHER_MAX_CANDIDATES,
        },
    )
    .candidates
    .into_iter()
    .map(|candidate| {
        let record = OpportunityRecord {
            tx_hash: candidate.tx_hash,
            strategy: format!("{:?}", candidate.strategy),
            score: candidate.score,
            protocol: candidate.protocol.clone(),
            category: candidate.category.clone(),
            feature_engine_version: candidate.feature_engine_version.clone(),
            scorer_version: candidate.scorer_version.clone(),
            strategy_version: candidate.strategy_version.clone(),
            reasons: candidate.reasons.clone(),
            detected_unix_ms,
            chain_id,
        };
        ExecutableOpportunity { record, candidate }
    })
    .collect()
}

fn sort_and_limit_opportunity_records(candidates: &mut Vec<OpportunityRecord>) {
    candidates.sort_unstable_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.strategy.cmp(&right.strategy))
            .then_with(|| left.tx_hash.cmp(&right.tx_hash))
    });
    candidates.truncate(SEARCHER_MAX_CANDIDATES);
}

fn opportunity_identity(candidate: &OpportunityRecord) -> (TxHash, String) {
    (candidate.tx_hash, candidate.strategy.clone())
}

fn ready_transactions_for_queue_transitions(
    scheduler: &SchedulerHandle,
    queue_transitions: &[SchedulerQueueTransition],
) -> Vec<ValidatedTransaction> {
    let mut ready_hashes = Vec::new();
    let mut seen = FastSet::default();

    for transition in queue_transitions {
        if matches!(transition.state, SchedulerQueueState::Ready) && seen.insert(transition.hash) {
            ready_hashes.push(transition.hash);
        }
    }

    if ready_hashes.is_empty() {
        return Vec::new();
    }

    scheduler.get_pending_transactions(&ready_hashes)
}

fn ready_frontier_for_queue_transitions(
    scheduler: &SchedulerHandle,
    queue_transitions: &[SchedulerQueueTransition],
) -> Vec<ValidatedTransaction> {
    if !queue_transitions
        .iter()
        .any(|transition| matches!(transition.state, SchedulerQueueState::Ready))
    {
        return Vec::new();
    }

    scheduler.snapshot().ready
}

fn build_opportunity_detected_payload(opportunity: &OpportunityRecord) -> EventPayload {
    EventPayload::OppDetected(OppDetected {
        hash: opportunity.tx_hash,
        strategy: opportunity.strategy.clone(),
        score: opportunity.score,
        protocol: opportunity.protocol.clone(),
        category: opportunity.category.clone(),
        feature_engine_version: opportunity.feature_engine_version.clone(),
        scorer_version: opportunity.scorer_version.clone(),
        strategy_version: opportunity.strategy_version.clone(),
        reasons: opportunity.reasons.clone(),
    })
}

fn build_simulation_batches(
    opportunities: &[ExecutableOpportunity],
    executable: &[ValidatedTransaction],
) -> Vec<(String, RemoteSimulationRequest, Vec<ExecutableOpportunity>)> {
    let mut batches =
        BTreeMap::<String, (RemoteSimulationRequest, Vec<ExecutableOpportunity>)>::new();

    for opportunity in opportunities {
        let Some(request) = build_remote_simulation_request(opportunity, executable) else {
            continue;
        };
        let task_key = simulation_task_key(&request);
        batches
            .entry(task_key)
            .and_modify(|(_, grouped)| grouped.push(opportunity.clone()))
            .or_insert_with(|| (request, vec![opportunity.clone()]));
    }

    batches
        .into_iter()
        .map(|(task_key, (request, grouped_opportunities))| {
            (task_key, request, grouped_opportunities)
        })
        .collect()
}

fn build_remote_simulation_request(
    opportunity: &ExecutableOpportunity,
    executable: &[ValidatedTransaction],
) -> Option<RemoteSimulationRequest> {
    let requested_hashes = if opportunity.candidate.member_tx_hashes.is_empty() {
        vec![opportunity.record.tx_hash]
    } else {
        opportunity.candidate.member_tx_hashes.clone()
    };
    let mut selected_hashes = requested_hashes
        .iter()
        .copied()
        .collect::<FastSet<TxHash>>();
    for requested_hash in &requested_hashes {
        let requested_tx = executable.iter().find(|tx| tx.hash() == *requested_hash)?;
        for candidate in executable {
            if candidate.decoded.sender == requested_tx.decoded.sender
                && candidate.decoded.nonce <= requested_tx.decoded.nonce
            {
                selected_hashes.insert(candidate.hash());
            }
        }
    }
    let txs = executable
        .iter()
        .filter(|tx| selected_hashes.contains(&tx.hash()))
        .cloned()
        .collect::<Vec<_>>();
    let member_tx_hashes = txs
        .iter()
        .map(ValidatedTransaction::hash)
        .collect::<Vec<_>>();

    Some(RemoteSimulationRequest {
        tx_hash: opportunity.record.tx_hash,
        member_tx_hashes,
        txs,
        detected_unix_ms: opportunity.record.detected_unix_ms,
    })
}

fn simulation_task_key(request: &RemoteSimulationRequest) -> String {
    let mut members = request
        .member_tx_hashes
        .iter()
        .map(|hash| format_fixed_hex(hash))
        .collect::<Vec<_>>();
    members.sort();
    format!(
        "{}:{}",
        format_fixed_hex(&request.tx_hash),
        members.join(",")
    )
}

async fn simulate_remote_request(
    client: &reqwest::Client,
    chain: &ChainRpcConfig,
    request: &RemoteSimulationRequest,
) -> Result<RemoteSimulationOutcome> {
    let hash_prefix = format_fixed_hex(&request.tx_hash)[2..10].to_owned();
    let sim_id = format!("sim-{hash_prefix}-{}", request.detected_unix_ms);
    let started = Instant::now();
    let tx_count = request.txs.len();
    let timeout = Duration::from_millis(resolve_sim_rpc_timeout_ms());
    let simulated = tokio::time::timeout(timeout, async {
        let http_url = chain
            .primary_http_url()
            .ok_or_else(|| anyhow!("no http endpoint configured for simulation"))?;
        let chain_context = fetch_remote_chain_context(client, chain, http_url, request).await?;
        let account_seeds =
            fetch_cached_account_seeds(client, http_url, chain_context.block_number, &request.txs)
                .await?;
        let provider = CachedStateProviderView { account_seeds };
        let inputs = request
            .txs
            .iter()
            .map(|tx| SimulationTxInput {
                decoded: tx.decoded.clone(),
                calldata: Some(tx.calldata.clone()),
            })
            .collect::<Vec<_>>();
        simulate_with_mode(
            &chain_context,
            &inputs,
            SimulationMode::RpcBacked(&provider),
        )
    })
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;

    let (status, fail_category, simulation_batch) = match simulated {
        Ok(Ok(batch)) => {
            let (status, fail_category) = summarize_simulation_batch(&batch);
            (status, fail_category, Some(batch))
        }
        Ok(Err(_)) => (
            RemoteSimulationStatus::StateError,
            Some("state_rpc".to_owned()),
            None,
        ),
        Err(_) => (
            RemoteSimulationStatus::Timeout,
            Some("state_timeout".to_owned()),
            None,
        ),
    };
    live_rpc_simulation_metrics().observe_result(
        status,
        latency_ms,
        tx_count,
        fail_category.as_deref(),
    );

    Ok(RemoteSimulationOutcome {
        sim_id,
        status,
        fail_category,
        latency_ms,
        tx_count: tx_count as u32,
        simulation_batch,
    })
}

fn build_bundle_submitted_payload(
    opportunity: &OpportunityRecord,
    sim_event: &SimCompleted,
) -> EventPayload {
    EventPayload::BundleSubmitted(BundleSubmitted {
        hash: opportunity.tx_hash,
        bundle_id: bundle_id_for_opportunity(opportunity),
        sim_id: sim_event.sim_id.clone(),
        relay: "not_submitted".to_owned(),
        accepted: false,
        feature_engine_version: opportunity.feature_engine_version.clone(),
        scorer_version: opportunity.scorer_version.clone(),
        strategy_version: opportunity.strategy_version.clone(),
    })
}

fn build_sim_completed_payload(
    opportunity: &OpportunityRecord,
    outcome: &RemoteSimulationOutcome,
) -> SimCompleted {
    SimCompleted {
        hash: opportunity.tx_hash,
        sim_id: outcome.sim_id.clone(),
        status: simulation_status_label(outcome.status).to_owned(),
        feature_engine_version: opportunity.feature_engine_version.clone(),
        scorer_version: opportunity.scorer_version.clone(),
        strategy_version: opportunity.strategy_version.clone(),
        fail_category: outcome.fail_category.clone(),
        latency_ms: Some(outcome.latency_ms),
        tx_count: Some(outcome.tx_count),
    }
}

fn bundle_id_for_opportunity(opportunity: &OpportunityRecord) -> String {
    build_bundle_id(opportunity.tx_hash, opportunity.detected_unix_ms)
}

fn build_bundle_id(hash: TxHash, detected_unix_ms: i64) -> String {
    let hash_prefix = format_fixed_hex(&hash)[2..10].to_owned();
    format!("bundle-{hash_prefix}-{detected_unix_ms}")
}

fn summarize_simulation_batch(
    batch: &sim_engine::SimulationBatchResult,
) -> (RemoteSimulationStatus, Option<String>) {
    if let Some(failed) = batch.tx_results.iter().find(|result| !result.success) {
        let category = failed
            .fail_category
            .map(simulation_fail_category_label)
            .unwrap_or("unknown")
            .to_owned();
        return (RemoteSimulationStatus::Failed, Some(category));
    }

    (RemoteSimulationStatus::Ok, None)
}

fn simulation_status_label(status: RemoteSimulationStatus) -> &'static str {
    match status {
        RemoteSimulationStatus::Ok => "ok",
        RemoteSimulationStatus::Failed => "failed",
        RemoteSimulationStatus::StateError => "state_error",
        RemoteSimulationStatus::Timeout => "timeout",
    }
}

fn simulation_fail_category_label(category: SimulationFailCategory) -> &'static str {
    match category {
        SimulationFailCategory::Revert => "revert",
        SimulationFailCategory::OutOfGas => "out_of_gas",
        SimulationFailCategory::NonceMismatch => "nonce_mismatch",
        SimulationFailCategory::StateMismatch => "state_mismatch",
        SimulationFailCategory::Unknown => "unknown",
    }
}

fn resolve_sim_cache_ttl_ms() -> u64 {
    std::env::var(ENV_SIM_CACHE_TTL_MS)
        .ok()
        .as_deref()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SIM_CACHE_TTL_MS)
}

fn resolve_sim_rpc_timeout_ms() -> u64 {
    std::env::var(ENV_SIM_RPC_TIMEOUT_MS)
        .ok()
        .as_deref()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SIM_RPC_TIMEOUT_MS)
}

async fn fetch_remote_chain_context(
    client: &reqwest::Client,
    chain: &ChainRpcConfig,
    http_url: &str,
    request: &RemoteSimulationRequest,
) -> Result<ChainContext> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 44,
        "method": "eth_getBlockByNumber",
        "params": ["latest", false],
    });

    let response: RpcHeaderResponseEnvelope = client
        .post(http_url)
        .json(&request_body)
        .send()
        .await
        .with_context(|| format!("POST {http_url}"))?
        .error_for_status()
        .context("rpc http status error")?
        .json()
        .await
        .context("decode simulation header response")?;

    if let Some(error) = response.error {
        return Err(anyhow!(
            "rpc header fetch failed: {}",
            error
                .message
                .unwrap_or_else(|| "unknown rpc error".to_owned())
        ));
    }
    let header = response
        .result
        .ok_or_else(|| anyhow!("rpc header response missing result"))?;
    let chain_id = request
        .txs
        .first()
        .and_then(|tx| tx.decoded.chain_id)
        .or(chain.chain_id)
        .unwrap_or(1);
    let block_number =
        parse_hex_u64(&header.number).ok_or_else(|| anyhow!("invalid block number"))?;
    let block_timestamp =
        parse_hex_u64(&header.timestamp).ok_or_else(|| anyhow!("invalid block timestamp"))?;
    let gas_limit =
        parse_hex_u64(&header.gas_limit).ok_or_else(|| anyhow!("invalid block gas limit"))?;
    let base_fee_wei = header
        .base_fee_per_gas
        .as_deref()
        .and_then(parse_hex_u128)
        .unwrap_or_default();
    let coinbase = header
        .miner
        .as_deref()
        .and_then(parse_fixed_hex::<20>)
        .unwrap_or([0_u8; 20]);
    let state_root = header
        .state_root
        .as_deref()
        .and_then(parse_fixed_hex::<32>)
        .unwrap_or([0_u8; 32]);

    Ok(ChainContext {
        chain_id,
        block_number,
        block_timestamp,
        gas_limit,
        base_fee_wei,
        coinbase,
        state_root,
    })
}

async fn fetch_cached_account_seeds(
    client: &reqwest::Client,
    http_url: &str,
    block_number: u64,
    txs: &[ValidatedTransaction],
) -> Result<HashMap<Address, AccountSeed>> {
    let ttl_ms = resolve_sim_cache_ttl_ms() as i64;
    let now_unix_ms = current_unix_ms();
    let mut seeds = HashMap::new();
    let mut unique_senders = BTreeSet::new();
    for tx in txs {
        unique_senders.insert(tx.decoded.sender);
    }

    for sender in unique_senders {
        let cache_key = (http_url.to_owned(), sender);
        let cached = live_rpc_simulation_cache()
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .account_seeds
            .get(&cache_key)
            .copied();
        if let Some(entry) = cached
            && entry.block_number == block_number
            && now_unix_ms.saturating_sub(entry.cached_at_unix_ms) <= ttl_ms
        {
            live_rpc_simulation_metrics().observe_cache_hit();
            seeds.insert(sender, entry.seed);
            continue;
        }

        live_rpc_simulation_metrics().observe_cache_miss();
        let seed = fetch_account_seed_from_rpc(client, http_url, sender).await?;
        live_rpc_simulation_cache()
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .account_seeds
            .insert(
                cache_key,
                CachedAccountSeed {
                    seed,
                    block_number,
                    cached_at_unix_ms: now_unix_ms,
                },
            );
        seeds.insert(sender, seed);
    }

    Ok(seeds)
}

async fn fetch_account_seed_from_rpc(
    client: &reqwest::Client,
    http_url: &str,
    address: Address,
) -> Result<AccountSeed> {
    let address_hex = format_fixed_hex(&address);
    let balance = fetch_rpc_scalar(
        client,
        http_url,
        45,
        "eth_getBalance",
        json!([address_hex, "latest"]),
    )
    .await
    .and_then(|raw| parse_hex_u128(&raw).ok_or_else(|| anyhow!("invalid balance payload")))?;
    let nonce = fetch_rpc_scalar(
        client,
        http_url,
        46,
        "eth_getTransactionCount",
        json!([address_hex, "latest"]),
    )
    .await
    .and_then(|raw| parse_hex_u64(&raw).ok_or_else(|| anyhow!("invalid nonce payload")))?;

    Ok(AccountSeed {
        balance_wei: balance,
        nonce,
    })
}

async fn fetch_rpc_scalar(
    client: &reqwest::Client,
    http_url: &str,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> Result<String> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let response: RpcScalarResponseEnvelope = client
        .post(http_url)
        .json(&request_body)
        .send()
        .await
        .with_context(|| format!("POST {http_url}"))?
        .error_for_status()
        .context("rpc http status error")?
        .json()
        .await
        .context("decode rpc scalar response")?;
    if let Some(error) = response.error {
        return Err(anyhow!(
            "rpc scalar fetch failed: {}",
            error
                .message
                .unwrap_or_else(|| "unknown rpc error".to_owned())
        ));
    }
    response
        .result
        .ok_or_else(|| anyhow!("rpc scalar response missing result"))
}

struct CachedStateProviderView {
    account_seeds: HashMap<Address, AccountSeed>,
}

impl StateProvider for CachedStateProviderView {
    fn account_seed(&self, address: Address) -> Result<Option<AccountSeed>> {
        Ok(self.account_seeds.get(&address).copied())
    }
}

async fn rpc_post_bytes(
    client: &reqwest::Client,
    http_url: &str,
    body: &serde_json::Value,
) -> Result<Vec<u8>> {
    let bytes = client
        .post(http_url)
        .json(body)
        .send()
        .await
        .with_context(|| format!("POST {http_url}"))?
        .error_for_status()
        .context("rpc http status error")?
        .bytes()
        .await
        .context("read rpc response bytes")?;
    Ok(bytes.to_vec())
}

async fn fetch_transaction_by_hash(
    client: &reqwest::Client,
    http_url: &str,
    hash_hex: &str,
) -> Result<Option<LiveTx>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "eth_getTransactionByHash",
        "params": [hash_hex],
    });
    let response_bytes = rpc_post_bytes(client, http_url, &body).await?;
    decode_transaction_fetch_response_to_live_tx(response_bytes.as_ref(), hash_hex)
}

async fn fetch_pending_block_transactions(
    client: &reqwest::Client,
    http_url: &str,
) -> Result<Vec<LiveTx>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 43,
        "method": "eth_getBlockByNumber",
        "params": ["pending", true],
    });
    let response_bytes = rpc_post_bytes(client, http_url, &body).await?;
    decode_pending_block_response_to_live_txs(response_bytes.as_ref())
}

async fn fetch_transactions_by_hash_batch_with_retry(
    client: &reqwest::Client,
    http_url: &str,
    hashes: &[String],
    batch_fetch: BatchFetchConfig,
) -> Result<Vec<Option<LiveTx>>> {
    let mut attempt = 0usize;
    loop {
        match fetch_transactions_by_hash_batch(client, http_url, hashes).await {
            Ok(fetched) => return Ok(fetched),
            Err(err) => {
                if attempt >= batch_fetch.retry_attempts {
                    return Err(err.context(format!(
                        "fetch batched tx details failed after {} attempt(s)",
                        attempt + 1
                    )));
                }
                let delay_ms = retry_backoff_delay_ms(batch_fetch.retry_backoff_ms, attempt);
                tracing::warn!(
                    error = %err,
                    attempt = attempt + 1,
                    retry_in_ms = delay_ms,
                    batch_size = hashes.len(),
                    http_url,
                    "live rpc batched fetch failed; retrying"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                attempt += 1;
            }
        }
    }
}

async fn fetch_transactions_by_hash_batch(
    client: &reqwest::Client,
    http_url: &str,
    hashes: &[String],
) -> Result<Vec<Option<LiveTx>>> {
    if hashes.is_empty() {
        return Ok(Vec::new());
    }
    if hashes.len() == 1 {
        return Ok(vec![
            fetch_transaction_by_hash(client, http_url, hashes[0].as_str()).await?,
        ]);
    }

    let request_body = hashes
        .iter()
        .enumerate()
        .map(|(index, hash_hex)| {
            json!({
                "jsonrpc": "2.0",
                "id": index as u64,
                "method": "eth_getTransactionByHash",
                "params": [hash_hex],
            })
        })
        .collect::<Vec<_>>();

    let response_bytes =
        rpc_post_bytes(client, http_url, &serde_json::Value::Array(request_body)).await?;
    decode_transaction_fetch_batch_response_to_live_txs(response_bytes.as_ref(), hashes)
}

#[derive(Debug, Deserialize)]
struct RpcFetchErrorEnvelope {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcFetchResponseEnvelope {
    #[serde(default)]
    error: Option<RpcFetchErrorEnvelope>,
    #[serde(default)]
    result: Option<RpcTransaction>,
}

#[derive(Debug, Deserialize)]
struct RpcPendingBlock {
    #[serde(default)]
    transactions: Vec<RpcTransaction>,
}

#[derive(Debug, Deserialize)]
struct RpcPendingBlockResponseEnvelope {
    #[serde(default)]
    error: Option<RpcFetchErrorEnvelope>,
    #[serde(default)]
    result: Option<RpcPendingBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RpcBatchResponseId {
    Number(u64),
    String(String),
}

impl RpcBatchResponseId {
    fn to_index(&self) -> Option<usize> {
        let raw = match self {
            Self::Number(value) => Some(*value),
            Self::String(value) => {
                let trimmed = value.trim();
                if trimmed.starts_with("0x") {
                    parse_hex_u64(trimmed)
                } else {
                    trimmed.parse::<u64>().ok()
                }
            }
        }?;
        usize::try_from(raw).ok()
    }
}

#[derive(Debug, Deserialize)]
struct RpcBatchFetchResponseEnvelope {
    id: RpcBatchResponseId,
    #[serde(default)]
    error: Option<RpcFetchErrorEnvelope>,
    #[serde(default)]
    result: Option<RpcTransaction>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RpcBatchFetchResponseBody {
    Batch(Vec<RpcBatchFetchResponseEnvelope>),
    Single(Box<RpcBatchFetchResponseEnvelope>),
}

fn decode_transaction_fetch_response_to_live_tx(
    payload: &[u8],
    hash_hex: &str,
) -> Result<Option<LiveTx>> {
    let response: RpcFetchResponseEnvelope =
        serde_json::from_slice(payload).context("decode rpc json response")?;
    if let Some(error) = response.error {
        let code = error
            .code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        let message = error.message.unwrap_or_else(|| "unknown".to_owned());
        return Err(anyhow!("rpc returned error: code={code} message={message}"));
    }
    let tx = match response.result {
        Some(tx) => tx,
        None => return Ok(None),
    };
    Ok(Some(rpc_tx_to_live_tx(tx, hash_hex)?))
}

fn decode_pending_block_response_to_live_txs(payload: &[u8]) -> Result<Vec<LiveTx>> {
    let response: RpcPendingBlockResponseEnvelope =
        serde_json::from_slice(payload).context("decode pending block rpc json response")?;
    if let Some(error) = response.error {
        let code = error
            .code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        let message = error.message.unwrap_or_else(|| "unknown".to_owned());
        return Err(anyhow!("rpc returned error: code={code} message={message}"));
    }

    let Some(block) = response.result else {
        return Ok(Vec::new());
    };

    block
        .transactions
        .into_iter()
        .map(|tx| {
            let hash_hex = tx.hash.clone();
            rpc_tx_to_live_tx(tx, hash_hex.as_str())
        })
        .collect()
}

fn decode_transaction_fetch_batch_response_to_live_txs(
    payload: &[u8],
    hashes: &[String],
) -> Result<Vec<Option<LiveTx>>> {
    let response: RpcBatchFetchResponseBody =
        serde_json::from_slice(payload).context("decode rpc batch json response")?;
    let mut fetched = vec![None; hashes.len()];
    let envelopes = match response {
        RpcBatchFetchResponseBody::Batch(rows) => rows,
        RpcBatchFetchResponseBody::Single(row) => vec![*row],
    };

    for envelope in envelopes {
        let Some(index) = envelope.id.to_index() else {
            continue;
        };
        if index >= hashes.len() {
            continue;
        }

        if let Some(error) = envelope.error {
            let code = error
                .code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_owned());
            let message = error.message.unwrap_or_else(|| "unknown".to_owned());
            tracing::warn!(
                hash = hashes[index],
                code,
                message,
                "rpc batch transaction fetch returned item-level error"
            );
            continue;
        }

        let Some(tx) = envelope.result else {
            continue;
        };

        match rpc_tx_to_live_tx(tx, hashes[index].as_str()) {
            Ok(tx) => fetched[index] = Some(tx),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    hash = hashes[index],
                    "failed decoding transaction from rpc batch response"
                );
            }
        }
    }

    Ok(fetched)
}

pub fn decode_transaction_fetch_response(
    payload: &[u8],
    hash_hex: &str,
) -> Result<Option<TxDecoded>> {
    let live = match decode_transaction_fetch_response_to_live_tx(payload, hash_hex)? {
        Some(tx) => tx,
        None => return Ok(None),
    };
    Ok(Some(TxDecoded {
        hash: live.hash,
        tx_type: live.tx_type,
        sender: live.sender,
        nonce: live.nonce,
        chain_id: live.chain_id,
        to: live.to,
        value_wei: live.value_wei,
        gas_limit: live.gas_limit,
        gas_price_wei: live.gas_price_wei,
        max_fee_per_gas_wei: live.max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei: live.max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei: live.max_fee_per_blob_gas_wei,
        calldata_len: Some(live.input.len() as u32),
    }))
}

fn rpc_tx_to_live_tx(tx: RpcTransaction, hash_hex: &str) -> Result<LiveTx> {
    let hash = parse_fixed_hex::<32>(&tx.hash)
        .or_else(|| parse_fixed_hex::<32>(hash_hex))
        .ok_or_else(|| anyhow!("invalid transaction hash"))?;
    let sender = tx
        .from
        .as_deref()
        .and_then(parse_fixed_hex::<20>)
        .unwrap_or([0_u8; 20]);
    let to = tx.to.as_deref().and_then(parse_fixed_hex::<20>);
    let nonce = tx
        .nonce
        .as_deref()
        .and_then(parse_hex_u64)
        .unwrap_or_default();
    let tx_type = tx
        .tx_type
        .as_deref()
        .and_then(parse_hex_u64)
        .unwrap_or_default() as u8;
    let input = tx
        .input
        .as_deref()
        .and_then(parse_hex_bytes)
        .unwrap_or_default();
    let value_wei = tx.value.as_deref().and_then(parse_hex_u128);
    let gas_limit = tx.gas.as_deref().and_then(parse_hex_u64);
    let chain_id = tx.chain_id.as_deref().and_then(parse_hex_u64);
    let gas_price_wei = tx.gas_price.as_deref().and_then(parse_hex_u128);
    let max_fee_per_gas_wei = tx.max_fee_per_gas.as_deref().and_then(parse_hex_u128);
    let max_priority_fee_per_gas_wei = tx
        .max_priority_fee_per_gas
        .as_deref()
        .and_then(parse_hex_u128);
    let max_fee_per_blob_gas_wei = tx.max_fee_per_blob_gas.as_deref().and_then(parse_hex_u128);

    Ok(LiveTx {
        hash,
        sender,
        to,
        nonce,
        tx_type,
        value_wei,
        gas_limit,
        chain_id,
        gas_price_wei,
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei,
        input,
    })
}

pub(crate) fn parse_fixed_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = parse_hex_bytes(value)?;
    if bytes.len() != N {
        return None;
    }
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes);
    Some(out)
}

pub(crate) fn parse_hex_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    if !trimmed.len().is_multiple_of(2) {
        return None;
    }
    (0..trimmed.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&trimmed[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(trimmed, 16).ok()
}

fn parse_hex_u128(value: &str) -> Option<u128> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(0);
    }
    u128::from_str_radix(trimmed, 16).ok()
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut rendered = String::new();
    for (index, cause) in err.chain().enumerate() {
        if index > 0 {
            rendered.push_str(": ");
        }
        rendered.push_str(&cause.to_string());
    }
    rendered
}

pub(crate) fn format_fixed_hex(bytes: &[u8]) -> String {
    let mut out = String::from("0x");
    out.reserve(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn format_optional_fixed_hex(bytes: Option<&[u8]>) -> String {
    bytes
        .map(format_fixed_hex)
        .unwrap_or_else(|| "null".to_owned())
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn format_optional_u128(value: Option<u128>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn format_method_selector_hex(selector: Option<[u8; 4]>) -> String {
    match selector {
        Some(selector) => format!(
            "0x{:02x}{:02x}{:02x}{:02x}",
            selector[0], selector[1], selector[2], selector[3]
        ),
        None => "null".to_owned(),
    }
}

fn current_seq_hi(storage: &Arc<RwLock<InMemoryStorage>>) -> u64 {
    storage
        .read()
        .ok()
        .and_then(|store| store.latest_seq_id())
        .unwrap_or(0)
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use axum::Json;
    use axum::extract::State;
    use axum::routing::post;
    use axum::{Router, serve};
    use event_log::{TxBlocked, TxDropped, TxReady, TxReplaced};
    use serde_json::json;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU64, AtomicUsize};
    use tokio::net::TcpListener;

    static LIVE_RPC_TEST_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    fn live_rpc_test_guard() -> std::sync::MutexGuard<'static, ()> {
        LIVE_RPC_TEST_MUTEX
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[derive(Clone)]
    struct MockSimulationRpcState {
        block_number: Arc<AtomicU64>,
        block_requests: Arc<AtomicUsize>,
        balance_requests: Arc<AtomicUsize>,
        nonce_requests: Arc<AtomicUsize>,
        fail_account_reads: bool,
        response_delay_ms: u64,
    }

    impl MockSimulationRpcState {
        fn new(block_number: u64) -> Self {
            Self {
                block_number: Arc::new(AtomicU64::new(block_number)),
                block_requests: Arc::new(AtomicUsize::new(0)),
                balance_requests: Arc::new(AtomicUsize::new(0)),
                nonce_requests: Arc::new(AtomicUsize::new(0)),
                fail_account_reads: false,
                response_delay_ms: 0,
            }
        }
    }

    async fn start_mock_simulation_rpc(
        state: MockSimulationRpcState,
    ) -> (
        MockSimulationRpcState,
        SocketAddr,
        tokio::task::JoinHandle<()>,
    ) {
        let app = Router::new()
            .route("/", post(mock_simulation_rpc))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            serve(listener, app).await.unwrap();
        });

        (state, addr, server)
    }

    async fn mock_simulation_rpc(
        State(state): State<MockSimulationRpcState>,
        Json(payload): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        if state.response_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(state.response_delay_ms)).await;
        }
        let method = payload
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        match method {
            "eth_getBlockByNumber" => {
                state.block_requests.fetch_add(1, Ordering::SeqCst);
                let block_number = state.block_number.load(Ordering::SeqCst);
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": payload.get("id").cloned().unwrap_or(json!(1)),
                    "result": {
                        "number": format!("0x{block_number:x}"),
                        "timestamp": "0x65",
                        "gasLimit": "0x1c9c380",
                        "baseFeePerGas": "0x5",
                        "miner": format!("0x{}", "11".repeat(20)),
                        "stateRoot": format!("0x{}", "22".repeat(32)),
                    }
                }))
            }
            "eth_getBalance" => {
                state.balance_requests.fetch_add(1, Ordering::SeqCst);
                if state.fail_account_reads {
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": payload.get("id").cloned().unwrap_or(json!(1)),
                        "error": {
                            "code": -32000,
                            "message": "balance unavailable"
                        }
                    }))
                } else {
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": payload.get("id").cloned().unwrap_or(json!(1)),
                        "result": "0x3635c9adc5dea00000"
                    }))
                }
            }
            "eth_getTransactionCount" => {
                state.nonce_requests.fetch_add(1, Ordering::SeqCst);
                if state.fail_account_reads {
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": payload.get("id").cloned().unwrap_or(json!(1)),
                        "error": {
                            "code": -32000,
                            "message": "nonce unavailable"
                        }
                    }))
                } else {
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": payload.get("id").cloned().unwrap_or(json!(1)),
                        "result": "0x7"
                    }))
                }
            }
            other => Json(json!({
                "jsonrpc": "2.0",
                "id": payload.get("id").cloned().unwrap_or(json!(1)),
                "error": {
                    "code": -32601,
                    "message": format!("unsupported method: {other}")
                }
            })),
        }
    }

    fn test_chain_with_http_url(http_url: String) -> ChainRpcConfig {
        ChainRpcConfig {
            chain_key: "eth-mainnet".to_owned(),
            chain_id: Some(1),
            source_id: SourceId::new("rpc-live"),
            endpoints: vec![RpcEndpoint {
                ws_url: "ws://127.0.0.1/unused".to_owned(),
                http_url,
            }],
        }
    }

    fn test_chain() -> ChainRpcConfig {
        test_chain_with_http_url(PRIMARY_PUBLIC_HTTP_URL.to_owned())
    }

    fn sample_live_tx(
        hash_seed: u8,
        sender_seed: u8,
        nonce: u64,
        max_fee_per_gas_wei: u128,
    ) -> LiveTx {
        LiveTx {
            hash: [hash_seed; 32],
            sender: [sender_seed; 20],
            to: Some([hash_seed.saturating_add(1); 20]),
            nonce,
            tx_type: 2,
            value_wei: Some(10),
            gas_limit: Some(21_000),
            chain_id: Some(1),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(max_fee_per_gas_wei),
            max_priority_fee_per_gas_wei: Some(3),
            max_fee_per_blob_gas_wei: None,
            input: vec![0xaa, 0xbb, 0xcc],
        }
    }

    fn sample_swap_live_tx(
        hash_seed: u8,
        sender_seed: u8,
        nonce: u64,
        max_fee_per_gas_wei: u128,
    ) -> LiveTx {
        let mut tx = sample_live_tx(hash_seed, sender_seed, nonce, max_fee_per_gas_wei);
        tx.to = Some([
            0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac,
            0xb4, 0xc6, 0x59, 0xf2, 0x48, 0x8d,
        ]);
        tx.gas_limit = Some(320_000);
        tx.max_priority_fee_per_gas_wei = Some(7_000_000_000);
        tx.input = {
            let mut calldata = vec![0x38, 0xed, 0x17, 0x39];
            calldata.extend((0..160).map(|offset| offset as u8));
            calldata
        };
        tx
    }

    fn sample_pending_observation(
        hash: [u8; 32],
        observed_at_unix_ms: i64,
        observed_at_mono_ns: u64,
    ) -> PendingHashObservation {
        PendingHashObservation {
            hash_hex: format_fixed_hex(&hash),
            observed_at_unix_ms,
            observed_at_mono_ns,
        }
    }

    fn drain_storage_ops(
        storage_rx: &mut tokio::sync::mpsc::Receiver<StorageWriteOp>,
    ) -> Vec<StorageWriteOp> {
        let mut ops = Vec::new();
        while let Ok(op) = storage_rx.try_recv() {
            ops.push(op);
        }
        ops
    }

    async fn drain_storage_ops_until_quiet(
        storage_rx: &mut tokio::sync::mpsc::Receiver<StorageWriteOp>,
        quiet_ms: u64,
    ) -> Vec<StorageWriteOp> {
        let mut ops = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_millis(quiet_ms), storage_rx.recv()).await {
                Ok(Some(op)) => ops.push(op),
                Ok(None) | Err(_) => break,
            }
        }
        ops
    }

    fn sample_executable_opportunity(
        tx: &ValidatedTransaction,
        detected_unix_ms: i64,
    ) -> ExecutableOpportunity {
        ExecutableOpportunity {
            record: OpportunityRecord {
                tx_hash: tx.hash(),
                chain_id: tx.decoded.chain_id,
                strategy: "SandwichCandidate".to_owned(),
                score: 10_000,
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                feature_engine_version: "feature-engine.test".to_owned(),
                scorer_version: "scorer.test".to_owned(),
                strategy_version: "strategy.test".to_owned(),
                reasons: vec!["test".to_owned()],
                detected_unix_ms,
            },
            candidate: OpportunityCandidate {
                tx_hash: tx.hash(),
                member_tx_hashes: vec![tx.hash()],
                strategy: searcher::StrategyKind::SandwichCandidate,
                feature_engine_version: "feature-engine.test".to_owned(),
                scorer_version: "scorer.test".to_owned(),
                strategy_version: "strategy.test".to_owned(),
                score: 10_000,
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                breakdown: searcher::OpportunityScoreBreakdown {
                    mev_component: 10_000,
                    urgency_component: 0,
                    structural_component: 0,
                    strategy_bonus: 0,
                },
                reasons: vec!["test".to_owned()],
            },
        }
    }

    #[test]
    fn default_live_rpc_config_has_primary_and_fallback_public_endpoints() {
        let config = LiveRpcConfig::default();
        assert_eq!(config.chains.len(), 1);
        let chain = &config.chains[0];
        assert_eq!(chain.chain_key, "eth-mainnet");
        assert_eq!(chain.chain_id, Some(1));
        assert_eq!(chain.endpoints.len(), 2);
        assert_eq!(chain.endpoints[0].ws_url, PRIMARY_PUBLIC_WS_URL);
        assert_eq!(chain.endpoints[0].http_url, PRIMARY_PUBLIC_HTTP_URL);
        assert_eq!(chain.endpoints[1].ws_url, FALLBACK_PUBLIC_WS_URL);
        assert_eq!(chain.endpoints[1].http_url, FALLBACK_PUBLIC_HTTP_URL);
        assert_eq!(chain.source_id, SourceId::new("rpc-live"));
        assert!(config.max_seen_hashes >= 100);
    }

    #[test]
    fn format_error_chain_includes_context_and_root() {
        let err = anyhow!("root cause")
            .context("inner context")
            .context("outer context");

        let rendered = format_error_chain(&err);

        assert_eq!(rendered, "outer context: inner context: root cause");
    }

    #[test]
    fn rpc_tx_to_live_tx_parses_extended_fields() {
        let hash_hex = format!("0x{}", "11".repeat(32));
        let rpc_tx: RpcTransaction = serde_json::from_value(json!({
            "hash": hash_hex,
            "from": format!("0x{}", "22".repeat(20)),
            "to": format!("0x{}", "33".repeat(20)),
            "nonce": "0x2a",
            "type": "0x2",
            "input": "0xaabb",
            "value": "0x0de0b6b3a7640000",
            "gas": "0x5208",
            "chainId": "0x1",
            "gasPrice": "0x3b9aca00",
            "maxFeePerGas": "0x4a817c800",
            "maxPriorityFeePerGas": "0x77359400",
            "maxFeePerBlobGas": "0x3"
        }))
        .expect("decode rpc tx");

        let live = rpc_tx_to_live_tx(rpc_tx, &format!("0x{}", "11".repeat(32))).expect("map tx");

        assert_eq!(live.hash, [0x11; 32]);
        assert_eq!(live.sender, [0x22; 20]);
        assert_eq!(live.to, Some([0x33; 20]));
        assert_eq!(live.nonce, 42);
        assert_eq!(live.tx_type, 2);
        assert_eq!(live.input, vec![0xaa, 0xbb]);
        assert_eq!(live.value_wei, Some(1_000_000_000_000_000_000));
        assert_eq!(live.gas_limit, Some(21_000));
        assert_eq!(live.chain_id, Some(1));
        assert_eq!(live.gas_price_wei, Some(1_000_000_000));
        assert_eq!(live.max_fee_per_gas_wei, Some(20_000_000_000));
        assert_eq!(live.max_priority_fee_per_gas_wei, Some(2_000_000_000));
        assert_eq!(live.max_fee_per_blob_gas_wei, Some(3));
    }

    #[test]
    fn decode_pending_block_response_to_live_txs_maps_transactions() {
        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "transactions": [
                    {
                        "hash": format!("0x{}", "11".repeat(32)),
                        "from": format!("0x{}", "22".repeat(20)),
                        "to": format!("0x{}", "33".repeat(20)),
                        "nonce": "0x7",
                        "type": "0x2",
                        "input": "0xaabb",
                        "value": "0x1",
                        "gas": "0x5208",
                        "chainId": "0x1",
                        "maxFeePerGas": "0x64",
                        "maxPriorityFeePerGas": "0x3"
                    },
                    {
                        "hash": format!("0x{}", "44".repeat(32)),
                        "from": format!("0x{}", "55".repeat(20)),
                        "to": serde_json::Value::Null,
                        "nonce": "0x8",
                        "type": "0x2",
                        "input": "0xccdd",
                        "value": "0x2",
                        "gas": "0x7530",
                        "chainId": "0x1",
                        "maxFeePerGas": "0x96",
                        "maxPriorityFeePerGas": "0x4"
                    }
                ]
            }
        }))
        .expect("encode pending block payload");

        let txs =
            decode_pending_block_response_to_live_txs(&payload).expect("decode pending block");

        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].hash, [0x11; 32]);
        assert_eq!(txs[0].sender, [0x22; 20]);
        assert_eq!(txs[0].nonce, 7);
        assert_eq!(txs[1].hash, [0x44; 32]);
        assert_eq!(txs[1].sender, [0x55; 20]);
        assert_eq!(txs[1].nonce, 8);
    }

    #[test]
    fn build_opportunity_records_maps_ranked_candidates_to_storage_rows() {
        let uniswap_v2_router = [
            0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac,
            0xb4, 0xc6, 0x59, 0xf2, 0x48, 0x8d,
        ];
        let calldata = vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4, 5, 6, 7, 8];
        let decoded = TxDecoded {
            hash: [0x44; 32],
            tx_type: 2,
            sender: [0x55; 20],
            nonce: 42,
            chain_id: Some(1),
            to: Some(uniswap_v2_router),
            value_wei: Some(1_000_000_000_000_000),
            gas_limit: Some(320_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(45_000_000_000),
            max_priority_fee_per_gas_wei: Some(7_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(calldata.len() as u32),
        };

        let detected_unix_ms = 1_700_000_001_234;
        let records = build_opportunity_records(&decoded, &calldata, detected_unix_ms, Some(1));

        assert!(!records.is_empty());
        assert!(
            records
                .windows(2)
                .all(|pair| pair[0].score >= pair[1].score)
        );
        assert!(
            records
                .iter()
                .all(|record| record.detected_unix_ms == detected_unix_ms)
        );
        assert_eq!(records[0].feature_engine_version, feature_engine::version());
        assert_eq!(records[0].scorer_version, searcher::scorer_version());
        assert!(records[0].strategy_version.starts_with("strategy."));
        assert!(!records[0].reasons.is_empty());
    }

    #[test]
    fn build_opportunity_detected_payload_preserves_rule_versions() {
        let tx_hash = [0x11; 32];
        let opportunity = OpportunityRecord {
            tx_hash,
            chain_id: Some(1),
            strategy: "SandwichCandidate".to_owned(),
            score: 12_345,
            protocol: "uniswap-v2".to_owned(),
            category: "swap".to_owned(),
            feature_engine_version: "feature-engine.v9".to_owned(),
            scorer_version: "scorer.v3".to_owned(),
            strategy_version: "strategy.sandwich.v7".to_owned(),
            reasons: vec!["example-reason".to_owned()],
            detected_unix_ms: 1_700_000_001_000,
        };

        match build_opportunity_detected_payload(&opportunity) {
            EventPayload::OppDetected(opp) => {
                assert_eq!(opp.hash, tx_hash);
                assert_eq!(
                    opp.feature_engine_version,
                    opportunity.feature_engine_version
                );
                assert_eq!(opp.scorer_version, opportunity.scorer_version);
                assert_eq!(opp.strategy_version, opportunity.strategy_version);
            }
            other => panic!("expected OppDetected payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remote_simulation_request_uses_cache_until_block_number_changes() {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();

        let (rpc_state, rpc_addr, server) =
            start_mock_simulation_rpc(MockSimulationRpcState::new(100)).await;
        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let tx = validated_transaction_from_live_tx(
            &chain,
            1_700_000_000_001,
            101,
            &sample_swap_live_tx(0xc1, 0x41, 7, 120),
        );
        let request = build_remote_simulation_request(
            &sample_executable_opportunity(&tx, 1_700_000_000_010),
            &[tx.clone()],
        )
        .expect("simulation request");
        let client = simulation_http_client().clone();

        let first = simulate_remote_request(&client, &chain, &request)
            .await
            .expect("first simulation");
        let second = simulate_remote_request(&client, &chain, &request)
            .await
            .expect("second simulation");

        assert_eq!(first.status, RemoteSimulationStatus::Ok);
        assert_eq!(second.status, RemoteSimulationStatus::Ok);
        assert_eq!(rpc_state.balance_requests.load(Ordering::SeqCst), 1);
        assert_eq!(rpc_state.nonce_requests.load(Ordering::SeqCst), 1);

        rpc_state.block_number.store(101, Ordering::SeqCst);
        let third = simulate_remote_request(&client, &chain, &request)
            .await
            .expect("third simulation after block advance");

        assert_eq!(third.status, RemoteSimulationStatus::Ok);
        assert_eq!(rpc_state.balance_requests.load(Ordering::SeqCst), 2);
        assert_eq!(rpc_state.nonce_requests.load(Ordering::SeqCst), 2);

        server.abort();
    }

    #[tokio::test]
    async fn remote_simulation_request_classifies_rpc_state_errors() {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();

        let mut mock_state = MockSimulationRpcState::new(100);
        mock_state.fail_account_reads = true;
        let (_rpc_state, rpc_addr, server) = start_mock_simulation_rpc(mock_state).await;
        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let tx = validated_transaction_from_live_tx(
            &chain,
            1_700_000_000_001,
            101,
            &sample_swap_live_tx(0xc2, 0x42, 7, 120),
        );
        let request = build_remote_simulation_request(
            &sample_executable_opportunity(&tx, 1_700_000_000_010),
            &[tx],
        )
        .expect("simulation request");

        let outcome = simulate_remote_request(simulation_http_client(), &chain, &request)
            .await
            .expect("simulation outcome");

        assert_eq!(outcome.status, RemoteSimulationStatus::StateError);
        assert_eq!(outcome.fail_category.as_deref(), Some("state_rpc"));

        server.abort();
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_emits_real_sim_completed_payload() {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();
        reset_live_rpc_builder_runtime_state();

        let (rpc_state, rpc_addr, server) =
            start_mock_simulation_rpc(MockSimulationRpcState::new(100)).await;
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(128);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let tx = sample_swap_live_tx(0xc3, 0x43, 7, 120);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(tx.hash, 1_700_000_000_003, 303),
            tx.hash,
            Some(tx.clone()),
            &next_seq_id,
        )
        .await
        .expect("process tx");

        let ops = drain_storage_ops_until_quiet(&mut storage_rx, 250).await;
        let sim_payload = ops
            .iter()
            .find_map(|op| match op {
                StorageWriteOp::AppendPayload {
                    payload: EventPayload::SimCompleted(sim),
                    ..
                } if sim.hash == tx.hash => Some(sim.clone()),
                _ => None,
            })
            .expect("sim payload");

        assert_eq!(sim_payload.status, "ok");
        assert_eq!(sim_payload.fail_category, None);
        assert!(sim_payload.latency_ms.is_some());
        assert_eq!(sim_payload.tx_count, Some(1));

        let sim_count = ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    StorageWriteOp::AppendPayload {
                        payload: EventPayload::SimCompleted(sim),
                        ..
                    } if sim.hash == tx.hash
                )
            })
            .count() as u64;
        let metrics = live_rpc_simulation_metrics_snapshot();
        assert!(sim_count >= 1);
        assert_eq!(metrics.completed_total, 1);
        assert_eq!(metrics.ok_total, 1);
        assert_eq!(rpc_state.balance_requests.load(Ordering::SeqCst), 1);
        assert_eq!(rpc_state.nonce_requests.load(Ordering::SeqCst), 1);
        assert_eq!(live_rpc_builder_snapshot().candidates.len(), 1);
        assert_eq!(live_rpc_builder_metrics_snapshot().inserted_total, 1);

        runtime_task.abort();
        server.abort();
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_runs_simulation_in_downstream_worker_stage() {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();

        let mut mock_state = MockSimulationRpcState::new(100);
        mock_state.response_delay_ms = 25;
        let (_rpc_state, rpc_addr, server) = start_mock_simulation_rpc(mock_state).await;
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(256);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let tx = sample_swap_live_tx(0xc4, 0x44, 7, 120);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(tx.hash, 1_700_000_000_004, 404),
            tx.hash,
            Some(tx.clone()),
            &next_seq_id,
        )
        .await
        .expect("process tx");

        let immediate_ops = drain_storage_ops(&mut storage_rx);
        assert!(!immediate_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::SimCompleted(sim),
                ..
            } if sim.hash == tx.hash
        )));
        assert!(!immediate_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertOpportunity(record) if record.tx_hash == tx.hash
        )));

        let eventual_ops = drain_storage_ops_until_quiet(&mut storage_rx, 250).await;
        assert!(eventual_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::SimCompleted(sim),
                ..
            } if sim.hash == tx.hash
        )));
        assert!(eventual_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertOpportunity(record) if record.tx_hash == tx.hash
        )));

        runtime_task.abort();
        server.abort();
    }

    #[tokio::test]
    async fn scheduler_replacement_discards_stale_simulation_results() {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();

        let mut mock_state = MockSimulationRpcState::new(100);
        mock_state.response_delay_ms = 40;
        let (_rpc_state, rpc_addr, server) = start_mock_simulation_rpc(mock_state).await;
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(512);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let incumbent = sample_swap_live_tx(0xc5, 0x45, 7, 120);
        let replacement = sample_swap_live_tx(0xc6, 0x45, 7, 240);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(incumbent.hash, 1_700_000_000_005, 505),
            incumbent.hash,
            Some(incumbent.clone()),
            &next_seq_id,
        )
        .await
        .expect("process incumbent");

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(replacement.hash, 1_700_000_000_006, 606),
            replacement.hash,
            Some(replacement.clone()),
            &next_seq_id,
        )
        .await
        .expect("process replacement");

        let ops = drain_storage_ops_until_quiet(&mut storage_rx, 250).await;
        assert!(!ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::SimCompleted(sim),
                ..
            } if sim.hash == incumbent.hash
        )));
        assert!(!ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertOpportunity(record) if record.tx_hash == incumbent.hash
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::SimCompleted(sim),
                ..
            } if sim.hash == replacement.hash
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertOpportunity(record) if record.tx_hash == replacement.hash
        )));

        runtime_task.abort();
        server.abort();
    }

    #[test]
    fn validated_transaction_from_live_tx_preserves_scheduler_metadata() {
        let chain = test_chain();
        let tx = sample_live_tx(0x55, 0x22, 9, 120);

        let validated = validated_transaction_from_live_tx(&chain, 1_700_000_123_456, 999, &tx);

        assert_eq!(validated.source_id, SourceId::new("rpc-live"));
        assert_eq!(validated.observed_at_unix_ms, 1_700_000_123_456);
        assert_eq!(validated.observed_at_mono_ns, 999);
        assert_eq!(validated.decoded.hash, tx.hash);
        assert_eq!(validated.decoded.sender, tx.sender);
        assert_eq!(validated.decoded.nonce, tx.nonce);
        assert_eq!(validated.decoded.calldata_len, Some(3));
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_uses_pending_notification_timestamps_for_scheduler_admission()
     {
        let _guard = live_rpc_test_guard();
        let (storage_tx, _storage_rx) = tokio::sync::mpsc::channel(32);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain();
        let tx = sample_live_tx(0x66, 0x44, 12, 125);
        let observation = sample_pending_observation(tx.hash, 1_700_000_123_001, 555_444_333);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &observation,
            tx.hash,
            Some(tx.clone()),
            &next_seq_id,
        )
        .await
        .expect("process tx");

        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.pending.len(), 1);
        assert_eq!(snapshot.pending[0].decoded.hash, tx.hash);
        assert_eq!(
            snapshot.pending[0].observed_at_unix_ms,
            observation.observed_at_unix_ms
        );
        assert_eq!(
            snapshot.pending[0].observed_at_mono_ns,
            observation.observed_at_mono_ns
        );

        runtime_task.abort();
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_emits_replaced_event_for_scheduler_replacement() {
        let _guard = live_rpc_test_guard();
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(64);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain();
        let incumbent = sample_live_tx(0x71, 0x31, 8, 100);
        let replacement = sample_live_tx(0x72, 0x31, 8, 110);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(incumbent.hash, 1_700_000_000_001, 101),
            incumbent.hash,
            Some(incumbent.clone()),
            &next_seq_id,
        )
        .await
        .expect("process incumbent");
        let _ = drain_storage_ops(&mut storage_rx);

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(replacement.hash, 1_700_000_000_002, 202),
            replacement.hash,
            Some(replacement.clone()),
            &next_seq_id,
        )
        .await
        .expect("process replacement");

        let ops = drain_storage_ops(&mut storage_rx);
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxReplaced(TxReplaced { hash, replaced_by }), .. }
                if *hash == incumbent.hash && *replaced_by == replacement.hash
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxDecoded(decoded), .. }
                if decoded.hash == replacement.hash
        )));

        runtime_task.abort();
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_emits_dropped_event_when_scheduler_queue_is_full_but_still_writes_legacy_records()
     {
        let _guard = live_rpc_test_guard();
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(64);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, _runtime) = scheduler::scheduler_channel(scheduler::SchedulerConfig {
            handoff_queue_capacity: 1,
            max_pending_per_sender: 64,
            replacement_fee_bump_bps: 1_000,
        });

        let chain = test_chain();
        let tx = sample_live_tx(0x81, 0x41, 5, 150);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        scheduler
            .try_admit(validated_transaction_from_live_tx(
                &chain,
                1_700_000_000_000,
                10,
                &sample_live_tx(0x80, 0x40, 4, 100),
            ))
            .expect("fill scheduler handoff queue");

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(tx.hash, 1_700_000_000_003, 303),
            tx.hash,
            Some(tx.clone()),
            &next_seq_id,
        )
        .await
        .expect("queue full falls back to legacy comparison writes");

        let ops = drain_storage_ops(&mut storage_rx);
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertTxFull(record) if record.hash == tx.hash
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxDropped(TxDropped { hash, reason }), .. }
                if *hash == tx.hash && reason == "queue_full"
        )));
        assert!(!ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertOpportunity(record) if record.tx_hash == tx.hash
        )));
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_rejects_before_storage_when_scheduler_is_closed()
    {
        let _guard = live_rpc_test_guard();
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(8);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        drop(runtime);

        let chain = test_chain();
        let tx = sample_live_tx(0x71, 0x22, 9, 120);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        let error = process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(tx.hash, 1_700_000_000_010, 404),
            tx.hash,
            Some(tx),
            &next_seq_id,
        )
        .await
        .expect_err("closed scheduler should fail before storage writes");

        assert!(error.to_string().contains("scheduler"));
        assert!(storage_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_emits_queue_transition_events_for_gap_fill() {
        let _guard = live_rpc_test_guard();
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(64);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain();
        let nonce_7 = sample_live_tx(0x91, 0x31, 7, 100);
        let nonce_9 = sample_live_tx(0x92, 0x31, 9, 100);
        let nonce_8 = sample_live_tx(0x93, 0x31, 8, 100);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_7.hash, 1_700_000_000_001, 101),
            nonce_7.hash,
            Some(nonce_7.clone()),
            &next_seq_id,
        )
        .await
        .expect("process nonce 7");
        let _ = drain_storage_ops(&mut storage_rx);

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_9.hash, 1_700_000_000_002, 202),
            nonce_9.hash,
            Some(nonce_9.clone()),
            &next_seq_id,
        )
        .await
        .expect("process nonce 9");

        let blocked_ops = drain_storage_ops(&mut storage_rx);
        assert!(blocked_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxBlocked(TxBlocked { hash, expected_nonce, .. }), .. }
                if *hash == nonce_9.hash && *expected_nonce == Some(8)
        )));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_8.hash, 1_700_000_000_003, 303),
            nonce_8.hash,
            Some(nonce_8.clone()),
            &next_seq_id,
        )
        .await
        .expect("process nonce 8");

        let ready_ops = drain_storage_ops(&mut storage_rx);
        assert!(ready_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxReady(TxReady { hash, nonce, .. }), .. }
                if *hash == nonce_8.hash && *nonce == 8
        )));
        assert!(ready_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxReady(TxReady { hash, nonce, .. }), .. }
                if *hash == nonce_9.hash && *nonce == 9
        )));

        runtime_task.abort();
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_defers_opportunities_until_transactions_are_ready()
     {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();

        let (mock_state, rpc_addr, server) =
            start_mock_simulation_rpc(MockSimulationRpcState::new(100)).await;
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(128);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let nonce_7 = sample_live_tx(0xa1, 0x31, 7, 100);
        let nonce_9 = sample_swap_live_tx(0xa2, 0x31, 9, 120);
        let nonce_8 = sample_swap_live_tx(0xa3, 0x31, 8, 121);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_7.hash, 1_700_000_000_001, 101),
            nonce_7.hash,
            Some(nonce_7),
            &next_seq_id,
        )
        .await
        .expect("process nonce 7");
        let _ = drain_storage_ops(&mut storage_rx);

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_9.hash, 1_700_000_000_002, 202),
            nonce_9.hash,
            Some(nonce_9.clone()),
            &next_seq_id,
        )
        .await
        .expect("process nonce 9");

        let blocked_ops = drain_storage_ops(&mut storage_rx);
        assert!(blocked_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxBlocked(TxBlocked { hash, expected_nonce, .. }), .. }
                if *hash == nonce_9.hash && *expected_nonce == Some(8)
        )));
        assert!(!blocked_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::OppDetected(opp),
                ..
            } if opp.hash == nonce_9.hash
        )));
        assert!(!blocked_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::UpsertOpportunity(record) if record.tx_hash == nonce_9.hash
        )));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_8.hash, 1_700_000_000_003, 303),
            nonce_8.hash,
            Some(nonce_8.clone()),
            &next_seq_id,
        )
        .await
        .expect("process nonce 8");

        let ready_ops = drain_storage_ops_until_quiet(&mut storage_rx, 250).await;
        assert!(ready_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::OppDetected(opp),
                ..
            } if opp.hash == nonce_8.hash
        )));
        assert!(ready_ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload {
                payload: EventPayload::OppDetected(opp),
                ..
            } if opp.hash == nonce_9.hash
        )));
        assert!(mock_state.balance_requests.load(Ordering::SeqCst) >= 1);

        runtime_task.abort();
        server.abort();
    }

    #[tokio::test]
    async fn process_pending_hash_with_fetched_tx_records_internal_candidate_comparison_metrics() {
        let _guard = live_rpc_test_guard();
        reset_live_rpc_searcher_metrics();
        reset_live_rpc_simulation_metrics();
        reset_live_rpc_simulation_cache();
        reset_live_rpc_simulation_runtime_state();

        let (_mock_state, rpc_addr, server) =
            start_mock_simulation_rpc(MockSimulationRpcState::new(100)).await;

        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(128);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain_with_http_url(format!("http://{rpc_addr}"));
        let nonce_7 = sample_live_tx(0xb1, 0x31, 7, 100);
        let nonce_9 = sample_swap_live_tx(0xb2, 0x31, 9, 120);
        let nonce_8 = sample_swap_live_tx(0xb3, 0x31, 8, 121);
        let next_seq_id = Arc::new(AtomicU64::new(1));

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_7.hash, 1_700_000_000_011, 111),
            nonce_7.hash,
            Some(nonce_7),
            &next_seq_id,
        )
        .await
        .expect("process nonce 7");
        let _ = drain_storage_ops(&mut storage_rx);

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_9.hash, 1_700_000_000_012, 222),
            nonce_9.hash,
            Some(nonce_9),
            &next_seq_id,
        )
        .await
        .expect("process nonce 9");
        let _ = drain_storage_ops(&mut storage_rx);

        process_pending_hash_with_fetched_tx(
            &writer,
            &scheduler,
            &chain,
            &sample_pending_observation(nonce_8.hash, 1_700_000_000_013, 333),
            nonce_8.hash,
            Some(nonce_8),
            &next_seq_id,
        )
        .await
        .expect("process nonce 8");

        let metrics = live_rpc_searcher_metrics_snapshot();
        assert_eq!(metrics.executable_batches_total, 1);
        assert_eq!(metrics.legacy_shadow_batches_total, 1);
        assert_eq!(metrics.comparison_batches_total, 1);
        assert!(metrics.executable_candidates_total > metrics.legacy_shadow_candidates_total);
        assert!(metrics.executable_bundle_candidates_total >= 1);
        assert!(metrics.executable_only_candidates_total >= 1);
        assert_eq!(metrics.executable_top_score_wins_total, 1);
        assert_eq!(metrics.legacy_top_score_wins_total, 0);

        runtime_task.abort();
        server.abort();
    }

    #[tokio::test]
    async fn rebuild_scheduler_from_pending_transactions_rehydrates_scheduler_and_emits_events() {
        let _guard = live_rpc_test_guard();
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::channel(128);
        let writer = StorageWriteHandle::from_sender(storage_tx);
        let (scheduler, runtime) =
            scheduler::scheduler_channel(scheduler::SchedulerConfig::default());
        let runtime_task = tokio::spawn(runtime.run());

        let chain = test_chain();
        let pending = vec![
            sample_live_tx(0xa1, 0x31, 7, 100),
            sample_live_tx(0xa2, 0x31, 9, 100),
        ];
        let next_seq_id = Arc::new(AtomicU64::new(1));

        let rebuilt = rebuild_scheduler_from_pending_transactions(
            &writer,
            &scheduler,
            &chain,
            &pending,
            &next_seq_id,
        )
        .await
        .expect("rebuild scheduler from pending transactions");

        assert_eq!(rebuilt, pending.len());
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.pending.len(), 2);
        assert_eq!(
            snapshot
                .ready
                .iter()
                .map(ValidatedTransaction::hash)
                .collect::<Vec<_>>(),
            vec![pending[0].hash]
        );
        assert_eq!(
            snapshot
                .blocked
                .iter()
                .map(ValidatedTransaction::hash)
                .collect::<Vec<_>>(),
            vec![pending[1].hash]
        );

        let ops = drain_storage_ops(&mut storage_rx);
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxDecoded(decoded), .. }
                if decoded.hash == pending[0].hash
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            StorageWriteOp::AppendPayload { payload: EventPayload::TxBlocked(TxBlocked { hash, expected_nonce, .. }), .. }
                if *hash == pending[1].hash && *expected_nonce == Some(8)
        )));

        runtime_task.abort();
    }
}
