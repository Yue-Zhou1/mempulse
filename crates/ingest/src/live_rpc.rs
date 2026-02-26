use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

static NEXT_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default)]
pub struct LiveRpcConfig;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LiveRpcDropMetricsSnapshot {
    pub storage_queue_full: u64,
    pub storage_queue_closed: u64,
    pub invalid_pending_hash: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LiveRpcChainStatus {
    pub chain_key: String,
    pub state: String,
}

#[derive(Clone, Debug, Default)]
pub struct LiveRpcRuntimeState {
    chain_status: Arc<RwLock<Vec<LiveRpcChainStatus>>>,
    drop_metrics: Arc<RwLock<LiveRpcDropMetricsSnapshot>>,
}

impl LiveRpcRuntimeState {
    pub fn chain_status_snapshot(&self) -> Vec<LiveRpcChainStatus> {
        self.chain_status
            .read()
            .map(|rows| rows.clone())
            .unwrap_or_default()
    }

    pub fn drop_metrics_snapshot(&self) -> LiveRpcDropMetricsSnapshot {
        self.drop_metrics
            .read()
            .map(|metrics| metrics.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct LiveRpcRuntime {
    _config: LiveRpcConfig,
    instance_id: u64,
    state: Arc<LiveRpcRuntimeState>,
}

impl LiveRpcRuntime {
    pub fn new(config: LiveRpcConfig) -> Self {
        Self {
            _config: config,
            instance_id: NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed),
            state: Arc::new(LiveRpcRuntimeState::default()),
        }
    }

    pub fn instance_id(&self) -> u64 {
        self.instance_id
    }

    pub fn state(&self) -> Arc<LiveRpcRuntimeState> {
        self.state.clone()
    }
}
