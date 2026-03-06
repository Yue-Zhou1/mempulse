#![forbid(unsafe_code)]

use common::{SourceId, TxHash};
use event_log::TxDecoded;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatedTransaction {
    pub source_id: SourceId,
    pub observed_at_unix_ms: i64,
    pub observed_at_mono_ns: u64,
    pub decoded: TxDecoded,
}

impl ValidatedTransaction {
    pub fn hash(&self) -> TxHash {
        self.decoded.hash
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    pub handoff_queue_capacity: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            handoff_queue_capacity: 1_024,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerSnapshot {
    pub pending: Vec<ValidatedTransaction>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerMetrics {
    pub admitted_total: u64,
    pub duplicate_total: u64,
    pub queue_full_drop_total: u64,
    pub pending_total: usize,
    pub queue_depth: usize,
    pub handoff_queue_capacity: usize,
}

#[derive(Clone, Debug)]
pub struct SchedulerHandle {
    ingress_tx: mpsc::Sender<ValidatedTransaction>,
    shared: Arc<SharedState>,
}

impl SchedulerHandle {
    pub fn try_admit(&self, tx: ValidatedTransaction) -> Result<(), SchedulerEnqueueError> {
        match self.ingress_tx.try_send(tx) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.shared
                    .queue_full_drop_total
                    .fetch_add(1, Ordering::Relaxed);
                Err(SchedulerEnqueueError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SchedulerEnqueueError::QueueClosed),
        }
    }

    pub fn snapshot(&self) -> SchedulerSnapshot {
        let state = self
            .shared
            .state
            .read()
            .unwrap_or_else(|poison| poison.into_inner());
        SchedulerSnapshot {
            pending: state.pending.values().cloned().collect(),
        }
    }

    pub fn metrics(&self) -> SchedulerMetrics {
        let state = self
            .shared
            .state
            .read()
            .unwrap_or_else(|poison| poison.into_inner());
        let handoff_queue_capacity = self.ingress_tx.max_capacity();
        let queue_depth = handoff_queue_capacity.saturating_sub(self.ingress_tx.capacity());

        SchedulerMetrics {
            admitted_total: state.admitted_total,
            duplicate_total: state.duplicate_total,
            queue_full_drop_total: self.shared.queue_full_drop_total.load(Ordering::Relaxed),
            pending_total: state.pending.len(),
            queue_depth,
            handoff_queue_capacity,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerEnqueueError {
    QueueFull,
    QueueClosed,
}

#[derive(Debug)]
pub struct SchedulerRuntime {
    ingress_rx: mpsc::Receiver<ValidatedTransaction>,
    shared: Arc<SharedState>,
}

impl SchedulerRuntime {
    pub async fn run(mut self) {
        while let Some(tx) = self.ingress_rx.recv().await {
            self.shared.admit(tx);
        }
    }
}

pub fn scheduler_channel(config: SchedulerConfig) -> (SchedulerHandle, SchedulerRuntime) {
    let config = SchedulerConfig {
        handoff_queue_capacity: config.handoff_queue_capacity.max(1),
    };
    let (ingress_tx, ingress_rx) = mpsc::channel(config.handoff_queue_capacity);
    let shared = Arc::new(SharedState {
        state: RwLock::new(SchedulerState::default()),
        queue_full_drop_total: AtomicU64::new(0),
    });

    (
        SchedulerHandle {
            ingress_tx,
            shared: Arc::clone(&shared),
        },
        SchedulerRuntime { ingress_rx, shared },
    )
}

pub fn spawn_scheduler(config: SchedulerConfig) -> SchedulerHandle {
    let (handle, runtime) = scheduler_channel(config);
    tokio::spawn(runtime.run());
    handle
}

#[derive(Debug)]
struct SharedState {
    state: RwLock<SchedulerState>,
    queue_full_drop_total: AtomicU64,
}

impl SharedState {
    fn admit(&self, tx: ValidatedTransaction) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        let hash = tx.hash();
        match state.pending.entry(hash) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(tx);
                state.admitted_total = state.admitted_total.saturating_add(1);
            }
            std::collections::btree_map::Entry::Occupied(_) => {
                state.duplicate_total = state.duplicate_total.saturating_add(1);
            }
        }
    }
}

#[derive(Debug, Default)]
struct SchedulerState {
    pending: BTreeMap<TxHash, ValidatedTransaction>,
    admitted_total: u64,
    duplicate_total: u64,
}
