use ahash::RandomState;
use anyhow::{Context, Result, anyhow};
use common::{SourceId, TxHash};
use event_log::{
    BundleSubmitted, EventPayload, OppDetected, SimCompleted, TxDecoded, TxFetched, TxSeen,
};
use feature_engine::{
    FeatureAnalysis, FeatureInput, analyze_transaction, version as feature_engine_version,
};
use futures::{SinkExt, StreamExt};
use hashbrown::HashSet;
use searcher::{SearcherConfig, SearcherInputTx, rank_opportunities};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
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
use tokio::sync::Semaphore;
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
const DEFAULT_SILENT_CHAIN_TIMEOUT_SECS: u64 = 20;
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

#[derive(Debug, Default)]
struct LiveRpcDropMetrics {
    storage_queue_full: AtomicU64,
    storage_queue_closed: AtomicU64,
    invalid_pending_hash: AtomicU64,
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

static LIVE_RPC_DROP_METRICS: OnceLock<Arc<LiveRpcDropMetrics>> = OnceLock::new();
static LIVE_RPC_FEED_START_COUNT: AtomicU64 = AtomicU64::new(0);

fn live_rpc_drop_metrics() -> &'static Arc<LiveRpcDropMetrics> {
    LIVE_RPC_DROP_METRICS.get_or_init(|| Arc::new(LiveRpcDropMetrics::default()))
}

pub fn live_rpc_drop_metrics_snapshot() -> LiveRpcDropMetricsSnapshot {
    live_rpc_drop_metrics().snapshot()
}

pub fn reset_live_rpc_drop_metrics() {
    live_rpc_drop_metrics().reset();
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
        let chains_override = std::env::var(ENV_CHAINS)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let ws_override = std::env::var(ENV_ETH_WS_URL)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let http_override = std::env::var(ENV_ETH_HTTP_URL)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let source_id_override = std::env::var(ENV_SOURCE_ID)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let max_seen_hashes = std::env::var(ENV_MAX_SEEN_HASHES)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
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

fn parse_env_usize(key: &str) -> Result<Option<usize>> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|err| anyhow!("invalid {key}: {err}"))
}

fn parse_env_u64(key: &str) -> Result<Option<u64>> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
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
    if let Some(path_override) = std::env::var(ENV_CHAIN_CONFIG_PATH)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
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
    config: LiveRpcConfig,
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
        handle.spawn(async move {
            run_chain_worker(
                writer,
                chain,
                max_seen_hashes,
                batch_fetch,
                silent_chain_timeout_secs,
                next_seq_id,
            )
            .await;
        });
    }
}

async fn run_chain_worker(
    writer: StorageWriteHandle,
    chain: ChainRpcConfig,
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

struct LiveRpcSessionContext<'a> {
    writer: &'a StorageWriteHandle,
    chain: &'a ChainRpcConfig,
    endpoint: &'a RpcEndpoint,
    endpoint_index: usize,
    max_seen_hashes: usize,
    batch_fetch: BatchFetchConfig,
    silent_chain_timeout_secs: u64,
    client: &'a reqwest::Client,
    next_seq_id: &'a Arc<AtomicU64>,
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
    let mut pending_hashes = VecDeque::new();
    let in_flight = Arc::new(Semaphore::new(session.batch_fetch.max_in_flight));
    let mut last_pending_unix_ms = current_unix_ms();
    loop {
        match tokio::time::timeout(flush_interval, read.next()).await {
            Ok(Some(frame)) => {
                let frame = frame.context("read websocket frame")?;
                match frame {
                    Message::Text(text) => {
                        if let Some(hash_hex) = parse_pending_hash(&text, &mut subscription_id) {
                            pending_hashes.push_back(hash_hex.to_owned());
                            last_pending_unix_ms = current_unix_ms();
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
    pending_hashes: &mut VecDeque<String>,
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
    hash_batch: Vec<String>,
    in_flight: &Arc<Semaphore>,
) -> Result<()> {
    let mut deduped_hashes = Vec::new();
    let mut deduped_raw = Vec::new();
    for hash_hex in hash_batch {
        let hash = match parse_fixed_hex::<32>(&hash_hex) {
            Some(hash) => hash,
            None => {
                observe_live_rpc_drop_reason(LiveRpcDropReason::InvalidPendingHash);
                tracing::warn!(
                    chain_key = %session.chain.chain_key,
                    source_id = %session.chain.source_id,
                    hash = %hash_hex,
                    reason = LiveRpcDropReason::InvalidPendingHash.as_label(),
                    "dropping pending hash: invalid hex payload"
                );
                continue;
            }
        };
        if remember_hash(hash, seen_hashes, seen_order, session.max_seen_hashes) {
            deduped_hashes.push(hash_hex);
            deduped_raw.push(hash);
        }
    }
    if deduped_hashes.is_empty() {
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
        &deduped_hashes,
        session.batch_fetch,
    )
    .await?;
    let rpc_latency_ms = rpc_started.elapsed().as_secs_f64() * 1_000.0;
    let fetched_count = fetched.iter().filter(|row| row.is_some()).count();
    let error_count = deduped_hashes.len().saturating_sub(fetched_count);
    let error_rate = error_count as f64 / deduped_hashes.len() as f64;
    tracing::info!(
        chain_key = %session.chain.chain_key,
        chain_id = ?session.chain.chain_id,
        batch_size = deduped_hashes.len(),
        fetched_count,
        error_count,
        error_rate,
        rpc_latency_ms,
        "live rpc batch fetch metrics"
    );

    for ((hash_hex, hash), fetched_tx) in deduped_hashes
        .iter()
        .zip(deduped_raw.into_iter())
        .zip(fetched.into_iter())
    {
        process_pending_hash_with_fetched_tx(
            session.writer,
            session.chain,
            hash_hex,
            hash,
            fetched_tx,
            session.next_seq_id,
        )?;
    }

    Ok(())
}

fn process_pending_hash_with_fetched_tx(
    writer: &StorageWriteHandle,
    chain: &ChainRpcConfig,
    hash_hex: &str,
    hash: TxHash,
    fetched_tx: Option<LiveTx>,
    next_seq_id: &Arc<AtomicU64>,
) -> Result<()> {
    let now_unix_ms = current_unix_ms();
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
            hash = hash_hex,
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
            hash = hash_hex,
            chain_key = %chain.chain_key,
            source_id = %chain.source_id,
            "mempool transaction (details unavailable from rpc)"
        );
    }

    let tx_seen_seq_id = next_seq_id.fetch_add(1, Ordering::Relaxed);
    let tx_seen_mono_ns = tx_seen_seq_id.saturating_mul(1_000_000);
    if !try_enqueue_storage_write(
        writer,
        chain,
        StorageWriteOp::AppendPayload {
            source_id: chain.source_id.clone(),
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: tx_seen_mono_ns,
            payload: EventPayload::TxSeen(TxSeen {
                hash,
                peer_id: "rpc-ws".to_owned(),
                seen_at_unix_ms: now_unix_ms,
                seen_at_mono_ns: tx_seen_mono_ns,
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
            first_seen_unix_ms: now_unix_ms,
            first_seen_mono_ns: tx_seen_mono_ns,
            seen_count: 1,
        }),
    )? {
        return Ok(());
    }

    if !append_event(
        writer,
        chain,
        next_seq_id,
        now_unix_ms,
        EventPayload::TxFetched(TxFetched {
            hash,
            fetched_at_unix_ms: now_unix_ms,
        }),
    )? {
        return Ok(());
    }

    if let Some(tx) = fetched_tx {
        let analysis = feature_analysis.unwrap_or(default_feature_analysis());
        let raw_tx = tx.input.clone();
        let decoded = TxDecoded {
            hash: tx.hash,
            tx_type: tx.tx_type,
            sender: tx.sender,
            nonce: tx.nonce,
            chain_id: resolved_chain_id,
            to: tx.to,
            value_wei: tx.value_wei,
            gas_limit: tx.gas_limit,
            gas_price_wei: tx.gas_price_wei,
            max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
            max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
            calldata_len: Some(raw_tx.len() as u32),
        };
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
        if !append_event(
            writer,
            chain,
            next_seq_id,
            now_unix_ms,
            EventPayload::TxDecoded(decoded.clone()),
        )? {
            return Ok(());
        }
        for opportunity in
            build_opportunity_records(&decoded, &raw_tx, now_unix_ms, resolved_chain_id)
        {
            let payloads = build_rule_versioned_payloads(&opportunity);
            for payload in payloads {
                if !append_event(writer, chain, next_seq_id, now_unix_ms, payload)? {
                    return Ok(());
                }
            }
            if !try_enqueue_storage_write(
                writer,
                chain,
                StorageWriteOp::UpsertOpportunity(opportunity),
            )? {
                return Ok(());
            }
        }
    }

    Ok(())
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
    rank_opportunities(
        &[SearcherInputTx {
            decoded: decoded.clone(),
            calldata: calldata.to_vec().into(),
        }],
        SearcherConfig {
            min_score: SEARCHER_MIN_SCORE,
            max_candidates: SEARCHER_MAX_CANDIDATES,
        },
    )
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

fn build_rule_versioned_payloads(opportunity: &OpportunityRecord) -> Vec<EventPayload> {
    let hash_prefix = format_fixed_hex(&opportunity.tx_hash)[2..10].to_owned();
    let sim_id = format!("sim-{hash_prefix}-{}", opportunity.detected_unix_ms);
    let bundle_id = format!("bundle-{hash_prefix}-{}", opportunity.detected_unix_ms);

    vec![
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
        }),
        EventPayload::SimCompleted(SimCompleted {
            hash: opportunity.tx_hash,
            sim_id: sim_id.clone(),
            status: "not_run".to_owned(),
            feature_engine_version: opportunity.feature_engine_version.clone(),
            scorer_version: opportunity.scorer_version.clone(),
            strategy_version: opportunity.strategy_version.clone(),
        }),
        EventPayload::BundleSubmitted(BundleSubmitted {
            hash: opportunity.tx_hash,
            bundle_id,
            sim_id,
            relay: "not_submitted".to_owned(),
            accepted: false,
            feature_engine_version: opportunity.feature_engine_version.clone(),
            scorer_version: opportunity.scorer_version.clone(),
            strategy_version: opportunity.strategy_version.clone(),
        }),
    ]
}

async fn fetch_transaction_by_hash(
    client: &reqwest::Client,
    http_url: &str,
    hash_hex: &str,
) -> Result<Option<LiveTx>> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "eth_getTransactionByHash",
        "params": [hash_hex],
    });

    let response_bytes = client
        .post(http_url)
        .json(&request_body)
        .send()
        .await
        .with_context(|| format!("POST {http_url}"))?
        .error_for_status()
        .context("rpc http status error")?
        .bytes()
        .await
        .context("read rpc response bytes")?;

    decode_transaction_fetch_response_to_live_tx(response_bytes.as_ref(), hash_hex)
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

    let response_bytes = client
        .post(http_url)
        .json(&request_body)
        .send()
        .await
        .with_context(|| format!("POST {http_url}"))?
        .error_for_status()
        .context("rpc http status error")?
        .bytes()
        .await
        .context("read rpc batch response bytes")?;

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

fn parse_fixed_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = parse_hex_bytes(value)?;
    if bytes.len() != N {
        return None;
    }
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn parse_hex_bytes(value: &str) -> Option<Vec<u8>> {
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

fn format_fixed_hex(bytes: &[u8]) -> String {
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
    use serde_json::json;

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
    fn build_rule_versioned_payloads_emit_opp_sim_and_bundle_with_same_versions() {
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

        let events = build_rule_versioned_payloads(&opportunity);
        assert_eq!(events.len(), 3);

        match &events[0] {
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

        match &events[1] {
            EventPayload::SimCompleted(sim) => {
                assert_eq!(sim.hash, tx_hash);
                assert_eq!(sim.status, "not_run");
                assert_eq!(
                    sim.feature_engine_version,
                    opportunity.feature_engine_version
                );
                assert_eq!(sim.scorer_version, opportunity.scorer_version);
                assert_eq!(sim.strategy_version, opportunity.strategy_version);
            }
            other => panic!("expected SimCompleted payload, got {other:?}"),
        }

        match &events[2] {
            EventPayload::BundleSubmitted(bundle) => {
                assert_eq!(bundle.hash, tx_hash);
                assert!(!bundle.accepted);
                assert_eq!(bundle.relay, "not_submitted");
                assert_eq!(
                    bundle.feature_engine_version,
                    opportunity.feature_engine_version
                );
                assert_eq!(bundle.scorer_version, opportunity.scorer_version);
                assert_eq!(bundle.strategy_version, opportunity.strategy_version);
            }
            other => panic!("expected BundleSubmitted payload, got {other:?}"),
        }
    }
}
