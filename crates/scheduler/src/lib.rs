#![forbid(unsafe_code)]

use common::{Address, SourceId, TxHash};
use event_log::TxDecoded;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};

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
    pub max_pending_per_sender: usize,
    // Basis points: 1_000 = 10.00%.
    pub replacement_fee_bump_bps: u16,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            handoff_queue_capacity: 1_024,
            max_pending_per_sender: 64,
            replacement_fee_bump_bps: 1_000,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SenderQueueSnapshot {
    pub sender: Address,
    pub queued: Vec<ValidatedTransaction>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerSnapshot {
    pub pending: Vec<ValidatedTransaction>,
    pub ready: Vec<ValidatedTransaction>,
    pub blocked: Vec<ValidatedTransaction>,
    pub sender_queues: Vec<SenderQueueSnapshot>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerMetrics {
    pub admitted_total: u64,
    pub duplicate_total: u64,
    pub replacement_total: u64,
    pub underpriced_replacement_total: u64,
    pub sender_limit_drop_total: u64,
    pub queue_full_drop_total: u64,
    pub pending_total: usize,
    pub ready_total: usize,
    pub blocked_total: usize,
    pub sender_total: usize,
    pub queue_depth: usize,
    pub handoff_queue_capacity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerAdmission {
    Admitted,
    Duplicate,
    Replaced { replaced_hash: TxHash },
    UnderpricedReplacement,
    SenderLimitReached,
}

impl SchedulerAdmission {
    pub fn is_admitted(self) -> bool {
        matches!(self, Self::Admitted | Self::Replaced { .. })
    }
}

#[derive(Debug)]
enum SchedulerCommand {
    Admit {
        tx: ValidatedTransaction,
        reply_tx: Option<oneshot::Sender<SchedulerAdmission>>,
    },
}

#[derive(Clone, Debug)]
pub struct SchedulerHandle {
    ingress_tx: mpsc::Sender<SchedulerCommand>,
    shared: Arc<SharedState>,
}

impl SchedulerHandle {
    pub async fn admit(
        &self,
        tx: ValidatedTransaction,
    ) -> Result<SchedulerAdmission, SchedulerEnqueueError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.try_send_command(SchedulerCommand::Admit {
            tx,
            reply_tx: Some(reply_tx),
        })?;
        reply_rx
            .await
            .map_err(|_| SchedulerEnqueueError::QueueClosed)
    }

    pub fn try_admit(&self, tx: ValidatedTransaction) -> Result<(), SchedulerEnqueueError> {
        self.try_send_command(SchedulerCommand::Admit { tx, reply_tx: None })
    }

    fn try_send_command(&self, command: SchedulerCommand) -> Result<(), SchedulerEnqueueError> {
        match self.ingress_tx.try_send(command) {
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
        state.snapshot()
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
            replacement_total: state.replacement_total,
            underpriced_replacement_total: state.underpriced_replacement_total,
            sender_limit_drop_total: state.sender_limit_drop_total,
            queue_full_drop_total: self.shared.queue_full_drop_total.load(Ordering::Relaxed),
            pending_total: state.pending.len(),
            ready_total: state.ready_total,
            blocked_total: state.blocked_total,
            sender_total: state.sender_queues.len(),
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
    ingress_rx: mpsc::Receiver<SchedulerCommand>,
    shared: Arc<SharedState>,
}

impl SchedulerRuntime {
    pub async fn run(mut self) {
        while let Some(command) = self.ingress_rx.recv().await {
            match command {
                SchedulerCommand::Admit { tx, reply_tx } => {
                    let result = self.shared.admit(tx);
                    if let Some(reply_tx) = reply_tx {
                        let _ = reply_tx.send(result);
                    }
                }
            }
        }
    }
}

pub fn scheduler_channel(config: SchedulerConfig) -> (SchedulerHandle, SchedulerRuntime) {
    let config = SchedulerConfig {
        handoff_queue_capacity: config.handoff_queue_capacity.max(1),
        max_pending_per_sender: config.max_pending_per_sender.max(1),
        replacement_fee_bump_bps: config.replacement_fee_bump_bps,
    };
    let (ingress_tx, ingress_rx) = mpsc::channel(config.handoff_queue_capacity);
    let shared = Arc::new(SharedState {
        config,
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
    let runtime_task = tokio::spawn(runtime.run());
    tokio::spawn(async move {
        match runtime_task.await {
            Ok(()) => tracing::warn!("scheduler runtime exited"),
            Err(error) => tracing::error!(?error, "scheduler runtime task failed"),
        }
    });
    handle
}

#[derive(Debug)]
struct SharedState {
    config: SchedulerConfig,
    state: RwLock<SchedulerState>,
    queue_full_drop_total: AtomicU64,
}

impl SharedState {
    fn admit(&self, tx: ValidatedTransaction) -> SchedulerAdmission {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        state.admit(tx, self.config)
    }
}

#[derive(Debug, Default)]
struct SchedulerState {
    pending: BTreeMap<TxHash, ValidatedTransaction>,
    sender_queues: BTreeMap<Address, BTreeMap<u64, TxHash>>,
    sender_queue_counts: BTreeMap<Address, SenderQueueCounts>,
    admitted_total: u64,
    duplicate_total: u64,
    replacement_total: u64,
    underpriced_replacement_total: u64,
    sender_limit_drop_total: u64,
    ready_total: usize,
    blocked_total: usize,
}

#[derive(Debug, Default)]
struct QueueClassification {
    ready: Vec<ValidatedTransaction>,
    blocked: Vec<ValidatedTransaction>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SenderQueueCounts {
    ready: usize,
    blocked: usize,
}

impl SchedulerState {
    fn snapshot(&self) -> SchedulerSnapshot {
        let classification = self.classify_by_sender();
        SchedulerSnapshot {
            pending: self.pending.values().cloned().collect(),
            ready: classification.ready,
            blocked: classification.blocked,
            sender_queues: self.sender_queue_snapshots(),
        }
    }

    fn admit(&mut self, tx: ValidatedTransaction, config: SchedulerConfig) -> SchedulerAdmission {
        let hash = tx.hash();
        if self.pending.contains_key(&hash) {
            self.duplicate_total = self.duplicate_total.saturating_add(1);
            return SchedulerAdmission::Duplicate;
        }

        let sender = tx.decoded.sender;
        let nonce = tx.decoded.nonce;

        if let Some(incumbent_hash) = self
            .sender_queues
            .get(&sender)
            .and_then(|queue| queue.get(&nonce).copied())
        {
            let Some(incumbent) = self.pending.get(&incumbent_hash) else {
                self.sender_queues
                    .entry(sender)
                    .or_default()
                    .insert(nonce, hash);
                self.pending.insert(hash, tx);
                self.admitted_total = self.admitted_total.saturating_add(1);
                self.refresh_sender_counts(sender);
                return SchedulerAdmission::Admitted;
            };
            if replacement_fee(
                tx.decoded.tx_type,
                tx.decoded.gas_price_wei,
                tx.decoded.max_fee_per_gas_wei,
            ) >= replacement_threshold_fee(incumbent, config.replacement_fee_bump_bps)
            {
                self.pending.remove(&incumbent_hash);
                self.sender_queues
                    .entry(sender)
                    .or_default()
                    .insert(nonce, hash);
                self.pending.insert(hash, tx);
                self.admitted_total = self.admitted_total.saturating_add(1);
                self.replacement_total = self.replacement_total.saturating_add(1);
                self.refresh_sender_counts(sender);
                return SchedulerAdmission::Replaced {
                    replaced_hash: incumbent_hash,
                };
            } else {
                self.underpriced_replacement_total =
                    self.underpriced_replacement_total.saturating_add(1);
                return SchedulerAdmission::UnderpricedReplacement;
            }
        }

        let sender_queue_len = self.sender_queues.get(&sender).map_or(0, BTreeMap::len);
        if sender_queue_len >= config.max_pending_per_sender {
            self.sender_limit_drop_total = self.sender_limit_drop_total.saturating_add(1);
            return SchedulerAdmission::SenderLimitReached;
        }

        self.sender_queues
            .entry(sender)
            .or_default()
            .insert(nonce, hash);
        self.pending.insert(hash, tx);
        self.admitted_total = self.admitted_total.saturating_add(1);
        self.refresh_sender_counts(sender);
        SchedulerAdmission::Admitted
    }

    fn classify_by_sender(&self) -> QueueClassification {
        let mut ready = Vec::new();
        let mut blocked = Vec::new();

        for queue in self.sender_queues.values() {
            let mut next_executable_nonce = None;
            let mut gap_seen = false;

            for (nonce, hash) in queue {
                let Some(tx) = self.pending.get(hash) else {
                    continue;
                };

                match next_executable_nonce {
                    None => {
                        ready.push(tx.clone());
                        next_executable_nonce = nonce.checked_add(1);
                    }
                    Some(expected) if !gap_seen && *nonce == expected => {
                        ready.push(tx.clone());
                        next_executable_nonce = nonce.checked_add(1);
                    }
                    Some(_) => {
                        gap_seen = true;
                        blocked.push(tx.clone());
                    }
                }
            }
        }

        QueueClassification { ready, blocked }
    }

    fn sender_queue_snapshots(&self) -> Vec<SenderQueueSnapshot> {
        self.sender_queues
            .iter()
            .map(|(sender, queue)| SenderQueueSnapshot {
                sender: *sender,
                queued: queue
                    .values()
                    .filter_map(|hash| self.pending.get(hash).cloned())
                    .collect(),
            })
            .collect()
    }

    fn refresh_sender_counts(&mut self, sender: Address) {
        let previous = self.sender_queue_counts.remove(&sender).unwrap_or_default();
        self.ready_total = self.ready_total.saturating_sub(previous.ready);
        self.blocked_total = self.blocked_total.saturating_sub(previous.blocked);

        let next = self
            .sender_queues
            .get(&sender)
            .map(|queue| self.classify_sender_queue_counts(queue))
            .unwrap_or_default();
        if next.ready != 0 || next.blocked != 0 {
            self.sender_queue_counts.insert(sender, next);
        }
        self.ready_total = self.ready_total.saturating_add(next.ready);
        self.blocked_total = self.blocked_total.saturating_add(next.blocked);
    }

    fn classify_sender_queue_counts(&self, queue: &BTreeMap<u64, TxHash>) -> SenderQueueCounts {
        let mut ready: usize = 0;
        let mut blocked: usize = 0;
        let mut next_executable_nonce = None;
        let mut gap_seen = false;

        for (nonce, hash) in queue {
            if !self.pending.contains_key(hash) {
                continue;
            }

            match next_executable_nonce {
                None => {
                    ready = ready.saturating_add(1);
                    next_executable_nonce = nonce.checked_add(1);
                }
                Some(expected) if !gap_seen && *nonce == expected => {
                    ready = ready.saturating_add(1);
                    next_executable_nonce = nonce.checked_add(1);
                }
                Some(_) => {
                    gap_seen = true;
                    blocked = blocked.saturating_add(1);
                }
            }
        }

        SenderQueueCounts { ready, blocked }
    }
}

fn replacement_threshold_fee(incumbent: &ValidatedTransaction, fee_bump_bps: u16) -> u128 {
    let current_fee = replacement_fee(
        incumbent.decoded.tx_type,
        incumbent.decoded.gas_price_wei,
        incumbent.decoded.max_fee_per_gas_wei,
    );
    current_fee
        .saturating_mul(10_000_u128.saturating_add(fee_bump_bps as u128))
        .div_ceil(10_000)
}

fn replacement_fee(
    tx_type: u8,
    gas_price_wei: Option<u128>,
    max_fee_per_gas_wei: Option<u128>,
) -> u128 {
    match tx_type {
        0 | 1 => gas_price_wei.unwrap_or_default(),
        _ => max_fee_per_gas_wei.or(gas_price_wei).unwrap_or_default(),
    }
}
