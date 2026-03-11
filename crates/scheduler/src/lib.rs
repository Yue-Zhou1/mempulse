#![forbid(unsafe_code)]

use common::{Address, CandidateId, SourceId, StrategyId, TxHash};
use event_log::TxDecoded;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatedTransaction {
    pub source_id: SourceId,
    pub observed_at_unix_ms: i64,
    pub observed_at_mono_ns: u64,
    #[serde(default)]
    pub calldata: Vec<u8>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SchedulerConfigError {
    #[error("handoff_queue_capacity must be >= 1, got 0")]
    HandoffQueueCapacityZero,
    #[error("max_pending_per_sender must be >= 1, got 0")]
    MaxPendingPerSenderZero,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SchedulerInitError {
    #[error(transparent)]
    Config(#[from] SchedulerConfigError),
    #[error(transparent)]
    Snapshot(#[from] SchedulerSnapshotError),
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerCandidate {
    pub candidate_id: CandidateId,
    pub tx_hash: TxHash,
    #[serde(default)]
    pub member_tx_hashes: Vec<TxHash>,
    pub score: u32,
    pub strategy: StrategyId,
    pub detected_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationTaskSpec {
    pub candidate_id: CandidateId,
    pub tx_hash: TxHash,
    #[serde(default)]
    pub member_tx_hashes: Vec<TxHash>,
    pub block_number: u64,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerCandidateDispatch {
    pub simulation_tasks: Vec<SimulationTaskSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerSimulationResult {
    pub candidate_id: CandidateId,
    pub tx_hash: TxHash,
    #[serde(default)]
    pub member_tx_hashes: Vec<TxHash>,
    pub block_number: u64,
    pub generation: u64,
    pub approved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerBuilderHandoff {
    pub candidate: SchedulerCandidate,
    pub block_number: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerSimulationApplyOutcome {
    pub builder_handoffs: Vec<SchedulerBuilderHandoff>,
    pub stale_result_drop_total: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SenderQueueSnapshot {
    pub sender: Address,
    pub queued: Vec<ValidatedTransaction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedSenderQueueEntry {
    pub nonce: u64,
    pub hash: TxHash,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedSenderQueueSnapshot {
    pub sender: Address,
    pub queued: Vec<PersistedSenderQueueEntry>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedSchedulerSnapshot {
    pub captured_at_unix_ms: i64,
    pub captured_at_mono_ns: u64,
    /// Filled by the caller with the event watermark captured alongside this
    /// snapshot. Scheduler-generated snapshots leave this as `0` until stamped.
    #[serde(default)]
    pub event_seq_hi: u64,
    pub pending: Vec<ValidatedTransaction>,
    pub executable_frontier: Vec<TxHash>,
    pub sender_queues: Vec<PersistedSenderQueueSnapshot>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerSnapshot {
    pub pending: Vec<ValidatedTransaction>,
    pub ready: Vec<ValidatedTransaction>,
    pub blocked: Vec<ValidatedTransaction>,
    pub sender_queues: Vec<SenderQueueSnapshot>,
    pub candidates: Vec<SchedulerCandidate>,
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
    pub stale_simulation_drop_total: u64,
    pub queue_depth: usize,
    pub queue_depth_peak: usize,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerQueueState {
    Ready,
    Blocked { expected_nonce: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerQueueTransition {
    pub hash: TxHash,
    pub sender: Address,
    pub nonce: u64,
    pub state: SchedulerQueueState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerAdmissionOutcome {
    pub admission: SchedulerAdmission,
    pub queue_transitions: Vec<SchedulerQueueTransition>,
}

#[derive(Debug)]
enum SchedulerCommand {
    Admit {
        tx: Box<ValidatedTransaction>,
        reply_tx: Option<oneshot::Sender<SchedulerAdmissionOutcome>>,
    },
    RegisterCandidates {
        candidates: Vec<SchedulerCandidate>,
        reply_tx: oneshot::Sender<SchedulerCandidateDispatch>,
    },
    ApplySimulationResult {
        result: SchedulerSimulationResult,
        reply_tx: oneshot::Sender<SchedulerSimulationApplyOutcome>,
    },
    InvalidateCandidateHash {
        hash: TxHash,
        reply_tx: oneshot::Sender<()>,
    },
    AdvanceHead {
        block_number: u64,
        reply_tx: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct SchedulerHandle {
    ingress_tx: mpsc::Sender<SchedulerCommand>,
    shared: Arc<SharedState>,
}

impl SchedulerHandle {
    #[must_use = "scheduler admission outcomes must be handled to observe enqueue failures"]
    #[inline]
    pub async fn admit_outcome(
        &self,
        tx: ValidatedTransaction,
    ) -> Result<SchedulerAdmissionOutcome, SchedulerEnqueueError> {
        self.send_command_with_reply(|reply_tx| SchedulerCommand::Admit {
            tx: Box::new(tx),
            reply_tx: Some(reply_tx),
        })
        .await
    }

    #[must_use = "scheduler admission results must be handled to observe enqueue failures"]
    #[inline]
    pub async fn admit(
        &self,
        tx: ValidatedTransaction,
    ) -> Result<SchedulerAdmission, SchedulerEnqueueError> {
        self.admit_outcome(tx)
            .await
            .map(|outcome| outcome.admission)
    }

    #[inline]
    pub fn try_admit(&self, tx: ValidatedTransaction) -> Result<(), SchedulerEnqueueError> {
        self.try_send_command(SchedulerCommand::Admit {
            tx: Box::new(tx),
            reply_tx: None,
        })
    }

    #[must_use = "candidate registration results must be handled to observe enqueue failures"]
    #[inline]
    pub async fn register_candidates(
        &self,
        candidates: Vec<SchedulerCandidate>,
    ) -> Result<SchedulerCandidateDispatch, SchedulerEnqueueError> {
        self.send_command_with_reply(|reply_tx| SchedulerCommand::RegisterCandidates {
            candidates,
            reply_tx,
        })
        .await
    }

    #[must_use = "simulation application results must be handled to observe enqueue failures"]
    #[inline]
    pub async fn apply_simulation_result(
        &self,
        result: SchedulerSimulationResult,
    ) -> Result<SchedulerSimulationApplyOutcome, SchedulerEnqueueError> {
        self.send_command_with_reply(|reply_tx| SchedulerCommand::ApplySimulationResult {
            result,
            reply_tx,
        })
        .await
    }

    #[must_use = "candidate invalidation results must be handled to observe enqueue failures"]
    #[inline]
    pub async fn invalidate_candidate_hash(
        &self,
        hash: TxHash,
    ) -> Result<(), SchedulerEnqueueError> {
        self.send_command_with_reply(|reply_tx| SchedulerCommand::InvalidateCandidateHash {
            hash,
            reply_tx,
        })
        .await
    }

    #[must_use = "head advancement results must be handled to observe enqueue failures"]
    #[inline]
    pub async fn advance_head(&self, block_number: u64) -> Result<(), SchedulerEnqueueError> {
        self.send_command_with_reply(|reply_tx| SchedulerCommand::AdvanceHead {
            block_number,
            reply_tx,
        })
        .await
    }

    async fn send_command_with_reply<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<T>) -> SchedulerCommand,
    ) -> Result<T, SchedulerEnqueueError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.try_send_command(build(reply_tx))?;
        reply_rx
            .await
            .map_err(|_| SchedulerEnqueueError::QueueClosed)
    }

    fn try_send_command(&self, command: SchedulerCommand) -> Result<(), SchedulerEnqueueError> {
        match self.ingress_tx.try_send(command) {
            Ok(()) => {
                let current_depth = self
                    .ingress_tx
                    .max_capacity()
                    .saturating_sub(self.ingress_tx.capacity());
                self.shared
                    .queue_depth_peak
                    .fetch_max(current_depth, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.shared
                    .queue_full_drop_total
                    .fetch_add(1, Ordering::Relaxed);
                self.shared
                    .queue_depth_peak
                    .fetch_max(self.ingress_tx.max_capacity(), Ordering::Relaxed);
                Err(SchedulerEnqueueError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SchedulerEnqueueError::QueueClosed),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> SchedulerSnapshot {
        let state = self.shared.state.read();
        state.snapshot()
    }

    pub fn get_pending_transactions(&self, hashes: &[TxHash]) -> Vec<ValidatedTransaction> {
        let state = self.shared.state.read();
        state.pending_transactions(hashes)
    }

    pub fn persisted_snapshot(
        &self,
        captured_at_unix_ms: i64,
        captured_at_mono_ns: u64,
    ) -> PersistedSchedulerSnapshot {
        // Callers must stamp `event_seq_hi` with the event watermark captured
        // alongside this snapshot before persisting it.
        let state = self.shared.state.read();
        state.persisted_snapshot(captured_at_unix_ms, captured_at_mono_ns)
    }

    pub fn metrics(&self) -> SchedulerMetrics {
        let state = self.shared.state.read();
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
            stale_simulation_drop_total: state.stale_simulation_drop_total,
            queue_depth,
            queue_depth_peak: self.shared.queue_depth_peak.load(Ordering::Relaxed),
            handoff_queue_capacity,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerEnqueueError {
    QueueFull,
    QueueClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SchedulerSnapshotError {
    #[error("duplicate pending transaction hash in snapshot: {hash:?}")]
    DuplicatePendingHash { hash: TxHash },
    #[error("missing pending transaction for sender queue entry: {hash:?}")]
    MissingPendingTransaction { hash: TxHash },
    #[error("duplicate sender nonce in snapshot: sender {sender:?}, nonce {nonce}")]
    DuplicateSenderNonce { sender: Address, nonce: u64 },
    #[error("sender queue entry does not match pending transaction sender/nonce: {hash:?}")]
    QueueEntryMismatch { hash: TxHash },
    #[error("snapshot executable frontier does not match reconstructed sender queues")]
    ExecutableFrontierMismatch,
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
                    let result = self.shared.admit(*tx);
                    if let Some(reply_tx) = reply_tx {
                        let _ = reply_tx.send(result);
                    }
                }
                SchedulerCommand::RegisterCandidates {
                    candidates,
                    reply_tx,
                } => {
                    let _ = reply_tx.send(self.shared.register_candidates(candidates));
                }
                SchedulerCommand::ApplySimulationResult { result, reply_tx } => {
                    let _ = reply_tx.send(self.shared.apply_simulation_result(result));
                }
                SchedulerCommand::InvalidateCandidateHash { hash, reply_tx } => {
                    self.shared.invalidate_candidate_hash(hash);
                    let _ = reply_tx.send(());
                }
                SchedulerCommand::AdvanceHead {
                    block_number,
                    reply_tx,
                } => {
                    self.shared.advance_head(block_number);
                    let _ = reply_tx.send(());
                }
            }
        }
    }
}

pub fn scheduler_channel(
    config: SchedulerConfig,
) -> Result<(SchedulerHandle, SchedulerRuntime), SchedulerConfigError> {
    let config = validate_config(config)?;
    Ok(scheduler_channel_with_state(
        config,
        SchedulerState::default(),
    ))
}

pub fn scheduler_channel_with_rehydration(
    config: SchedulerConfig,
    snapshot: Option<PersistedSchedulerSnapshot>,
    replay_transactions: Vec<ValidatedTransaction>,
) -> Result<(SchedulerHandle, SchedulerRuntime), SchedulerInitError> {
    let config = validate_config(config)?;
    let mut state = match snapshot {
        Some(snapshot) => SchedulerState::from_persisted_snapshot(snapshot)?,
        None => SchedulerState::default(),
    };
    for tx in replay_transactions {
        let _ = state.admit(tx, config);
    }
    Ok(scheduler_channel_with_state(config, state))
}

fn scheduler_channel_with_state(
    config: SchedulerConfig,
    state: SchedulerState,
) -> (SchedulerHandle, SchedulerRuntime) {
    let (ingress_tx, ingress_rx) = mpsc::channel(config.handoff_queue_capacity);
    let shared = Arc::new(SharedState {
        config,
        state: RwLock::new(state),
        queue_full_drop_total: AtomicU64::new(0),
        queue_depth_peak: AtomicUsize::new(0),
    });

    (
        SchedulerHandle {
            ingress_tx,
            shared: Arc::clone(&shared),
        },
        SchedulerRuntime { ingress_rx, shared },
    )
}

pub fn spawn_scheduler_with_rehydration(
    config: SchedulerConfig,
    snapshot: Option<PersistedSchedulerSnapshot>,
    replay_transactions: Vec<ValidatedTransaction>,
) -> Result<SchedulerHandle, SchedulerInitError> {
    let (handle, runtime) =
        scheduler_channel_with_rehydration(config, snapshot, replay_transactions)?;
    Ok(spawn_runtime(handle, runtime))
}

fn spawn_runtime(handle: SchedulerHandle, runtime: SchedulerRuntime) -> SchedulerHandle {
    let runtime_task = tokio::spawn(runtime.run());
    tokio::spawn(async move {
        match runtime_task.await {
            Ok(()) => tracing::warn!("scheduler runtime exited"),
            Err(error) => tracing::error!(?error, "scheduler runtime task failed"),
        }
    });
    handle
}

fn validate_config(config: SchedulerConfig) -> Result<SchedulerConfig, SchedulerConfigError> {
    if config.handoff_queue_capacity == 0 {
        return Err(SchedulerConfigError::HandoffQueueCapacityZero);
    }
    if config.max_pending_per_sender == 0 {
        return Err(SchedulerConfigError::MaxPendingPerSenderZero);
    }
    Ok(config)
}

fn normalized_member_hashes(tx_hash: TxHash, members: &[TxHash]) -> Vec<TxHash> {
    if members.is_empty() {
        vec![tx_hash]
    } else {
        members.to_vec()
    }
}

#[derive(Debug)]
struct SharedState {
    config: SchedulerConfig,
    state: RwLock<SchedulerState>,
    queue_full_drop_total: AtomicU64,
    queue_depth_peak: AtomicUsize,
}

impl SharedState {
    fn admit(&self, tx: ValidatedTransaction) -> SchedulerAdmissionOutcome {
        let mut state = self.state.write();
        state.admit(tx, self.config)
    }

    fn register_candidates(
        &self,
        candidates: Vec<SchedulerCandidate>,
    ) -> SchedulerCandidateDispatch {
        let mut state = self.state.write();
        state.register_candidates(candidates)
    }

    fn apply_simulation_result(
        &self,
        result: SchedulerSimulationResult,
    ) -> SchedulerSimulationApplyOutcome {
        let mut state = self.state.write();
        state.apply_simulation_result(result)
    }

    fn advance_head(&self, block_number: u64) {
        let mut state = self.state.write();
        state.advance_head(block_number);
    }

    fn invalidate_candidate_hash(&self, hash: TxHash) {
        let mut state = self.state.write();
        state.invalidate_candidate_hash(hash);
    }
}

#[derive(Debug)]
struct CandidateEntry {
    candidate: SchedulerCandidate,
    block_number: u64,
    generation: u64,
}

#[derive(Debug, Default)]
struct SchedulerState {
    pending: BTreeMap<TxHash, ValidatedTransaction>,
    sender_queues: BTreeMap<Address, BTreeMap<u64, TxHash>>,
    sender_queue_counts: BTreeMap<Address, SenderQueueCounts>,
    candidates: BTreeMap<CandidateId, CandidateEntry>,
    head_block_number: u64,
    admitted_total: u64,
    duplicate_total: u64,
    replacement_total: u64,
    underpriced_replacement_total: u64,
    sender_limit_drop_total: u64,
    stale_simulation_drop_total: u64,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SenderQueuePosition {
    hash: TxHash,
    nonce: u64,
    state: SchedulerQueueState,
}

impl SchedulerState {
    fn snapshot(&self) -> SchedulerSnapshot {
        let classification = self.classify_by_sender();
        SchedulerSnapshot {
            pending: self.pending.values().cloned().collect(),
            ready: classification.ready,
            blocked: classification.blocked,
            sender_queues: self.sender_queue_snapshots(),
            candidates: self
                .candidates
                .values()
                .map(|entry| entry.candidate.clone())
                .collect(),
        }
    }

    fn pending_transactions(&self, hashes: &[TxHash]) -> Vec<ValidatedTransaction> {
        hashes
            .iter()
            .filter_map(|hash| self.pending.get(hash).cloned())
            .collect()
    }

    fn persisted_snapshot(
        &self,
        captured_at_unix_ms: i64,
        captured_at_mono_ns: u64,
    ) -> PersistedSchedulerSnapshot {
        PersistedSchedulerSnapshot {
            captured_at_unix_ms,
            captured_at_mono_ns,
            event_seq_hi: 0,
            pending: self.pending.values().cloned().collect(),
            executable_frontier: self.executable_frontier_hashes(),
            sender_queues: self.persisted_sender_queue_snapshots(),
        }
    }

    fn from_persisted_snapshot(
        snapshot: PersistedSchedulerSnapshot,
    ) -> Result<Self, SchedulerSnapshotError> {
        let mut pending = BTreeMap::new();
        for tx in snapshot.pending {
            let hash = tx.hash();
            if pending.insert(hash, tx).is_some() {
                return Err(SchedulerSnapshotError::DuplicatePendingHash { hash });
            }
        }

        let sender_queues = if snapshot.sender_queues.is_empty() {
            build_sender_queues_from_pending(&pending)?
        } else {
            build_sender_queues_from_snapshot(&snapshot.sender_queues, &pending)?
        };

        let mut state = Self {
            pending,
            sender_queues,
            sender_queue_counts: BTreeMap::new(),
            candidates: BTreeMap::new(),
            head_block_number: 0,
            admitted_total: 0,
            duplicate_total: 0,
            replacement_total: 0,
            underpriced_replacement_total: 0,
            sender_limit_drop_total: 0,
            stale_simulation_drop_total: 0,
            ready_total: 0,
            blocked_total: 0,
        };
        state.recompute_queue_counts();

        let restored_frontier = state
            .classify_by_sender()
            .ready
            .iter()
            .map(ValidatedTransaction::hash)
            .collect::<Vec<_>>();
        if !snapshot.executable_frontier.is_empty()
            && restored_frontier != snapshot.executable_frontier
        {
            return Err(SchedulerSnapshotError::ExecutableFrontierMismatch);
        }

        Ok(state)
    }

    fn admit(
        &mut self,
        tx: ValidatedTransaction,
        config: SchedulerConfig,
    ) -> SchedulerAdmissionOutcome {
        let hash = tx.hash();
        if self.pending.contains_key(&hash) {
            self.duplicate_total = self.duplicate_total.saturating_add(1);
            return SchedulerAdmissionOutcome {
                admission: SchedulerAdmission::Duplicate,
                queue_transitions: Vec::new(),
            };
        }

        let sender = tx.decoded.sender;
        let nonce = tx.decoded.nonce;
        let previous_positions = self.sender_queue_positions(sender);

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
                return SchedulerAdmissionOutcome {
                    admission: SchedulerAdmission::Admitted,
                    queue_transitions: self.queue_transitions(sender, &previous_positions),
                };
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
                return SchedulerAdmissionOutcome {
                    admission: SchedulerAdmission::Replaced {
                        replaced_hash: incumbent_hash,
                    },
                    queue_transitions: self.queue_transitions(sender, &previous_positions),
                };
            } else {
                self.underpriced_replacement_total =
                    self.underpriced_replacement_total.saturating_add(1);
                return SchedulerAdmissionOutcome {
                    admission: SchedulerAdmission::UnderpricedReplacement,
                    queue_transitions: Vec::new(),
                };
            }
        }

        let sender_queue_len = self.sender_queues.get(&sender).map_or(0, BTreeMap::len);
        if sender_queue_len >= config.max_pending_per_sender {
            self.sender_limit_drop_total = self.sender_limit_drop_total.saturating_add(1);
            return SchedulerAdmissionOutcome {
                admission: SchedulerAdmission::SenderLimitReached,
                queue_transitions: Vec::new(),
            };
        }

        self.sender_queues
            .entry(sender)
            .or_default()
            .insert(nonce, hash);
        self.pending.insert(hash, tx);
        self.admitted_total = self.admitted_total.saturating_add(1);
        self.refresh_sender_counts(sender);
        SchedulerAdmissionOutcome {
            admission: SchedulerAdmission::Admitted,
            queue_transitions: self.queue_transitions(sender, &previous_positions),
        }
    }

    fn register_candidates(
        &mut self,
        candidates: Vec<SchedulerCandidate>,
    ) -> SchedulerCandidateDispatch {
        let simulation_tasks = candidates
            .into_iter()
            .map(|candidate| {
                let generation = self
                    .candidates
                    .get(&candidate.candidate_id)
                    .map(|entry| entry.generation.saturating_add(1).max(1))
                    .unwrap_or(1);
                let task = SimulationTaskSpec {
                    candidate_id: candidate.candidate_id.clone(),
                    tx_hash: candidate.tx_hash,
                    member_tx_hashes: normalized_member_hashes(
                        candidate.tx_hash,
                        &candidate.member_tx_hashes,
                    ),
                    block_number: self.head_block_number,
                    generation,
                };
                self.candidates.insert(
                    candidate.candidate_id.clone(),
                    CandidateEntry {
                        candidate,
                        block_number: self.head_block_number,
                        generation,
                    },
                );
                task
            })
            .collect();

        SchedulerCandidateDispatch { simulation_tasks }
    }

    fn apply_simulation_result(
        &mut self,
        result: SchedulerSimulationResult,
    ) -> SchedulerSimulationApplyOutcome {
        let Some(entry) = self.candidates.get(&result.candidate_id) else {
            return self.record_stale_simulation_result();
        };
        let expected_members =
            normalized_member_hashes(entry.candidate.tx_hash, &entry.candidate.member_tx_hashes);
        let actual_members = normalized_member_hashes(result.tx_hash, &result.member_tx_hashes);
        if entry.candidate.tx_hash != result.tx_hash
            || expected_members != actual_members
            || entry.block_number != result.block_number
            || entry.generation != result.generation
            || result.block_number != self.head_block_number
        {
            return self.record_stale_simulation_result();
        }

        let next_generation = entry.generation.saturating_add(1).max(1);
        let handoff = result.approved.then(|| SchedulerBuilderHandoff {
            candidate: entry.candidate.clone(),
            block_number: result.block_number,
        });

        if let Some(entry) = self.candidates.get_mut(&result.candidate_id) {
            entry.generation = next_generation;
        }

        SchedulerSimulationApplyOutcome {
            builder_handoffs: handoff.into_iter().collect(),
            stale_result_drop_total: 0,
        }
    }

    fn advance_head(&mut self, block_number: u64) {
        self.head_block_number = block_number;
    }

    fn invalidate_candidate_hash(&mut self, hash: TxHash) {
        let candidate_ids = self
            .candidates
            .iter()
            .filter_map(|(candidate_id, entry)| {
                let members = normalized_member_hashes(
                    entry.candidate.tx_hash,
                    &entry.candidate.member_tx_hashes,
                );
                (entry.candidate.tx_hash == hash || members.contains(&hash))
                    .then(|| candidate_id.clone())
            })
            .collect::<Vec<_>>();

        for candidate_id in candidate_ids {
            if let Some(entry) = self.candidates.get_mut(&candidate_id) {
                entry.generation = entry.generation.saturating_add(1).max(1);
            }
        }
    }

    fn record_stale_simulation_result(&mut self) -> SchedulerSimulationApplyOutcome {
        self.stale_simulation_drop_total = self.stale_simulation_drop_total.saturating_add(1);
        SchedulerSimulationApplyOutcome {
            builder_handoffs: Vec::new(),
            stale_result_drop_total: 1,
        }
    }

    fn classify_by_sender(&self) -> QueueClassification {
        let mut ready = Vec::with_capacity(self.ready_total);
        let mut blocked = Vec::with_capacity(self.blocked_total);

        for queue in self.sender_queues.values() {
            let mut next_executable_nonce = None;
            let mut gap_seen = false;

            for (nonce, hash) in queue {
                let Some(tx) = self.pending.get(hash) else {
                    continue;
                };
                match queue_entry_state(&mut next_executable_nonce, &mut gap_seen, *nonce) {
                    SchedulerQueueState::Ready => ready.push(tx.clone()),
                    SchedulerQueueState::Blocked { .. } => blocked.push(tx.clone()),
                }
            }
        }

        QueueClassification { ready, blocked }
    }

    fn executable_frontier_hashes(&self) -> Vec<TxHash> {
        let mut frontier = Vec::with_capacity(self.ready_total);

        for queue in self.sender_queues.values() {
            let mut next_executable_nonce = None;
            let mut gap_seen = false;

            for (nonce, hash) in queue {
                if !self.pending.contains_key(hash) {
                    continue;
                }
                if matches!(
                    queue_entry_state(&mut next_executable_nonce, &mut gap_seen, *nonce),
                    SchedulerQueueState::Ready
                ) {
                    frontier.push(*hash);
                }
            }
        }

        frontier
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

    fn persisted_sender_queue_snapshots(&self) -> Vec<PersistedSenderQueueSnapshot> {
        self.sender_queues
            .iter()
            .map(|(sender, queue)| PersistedSenderQueueSnapshot {
                sender: *sender,
                queued: queue
                    .iter()
                    .map(|(nonce, hash)| PersistedSenderQueueEntry {
                        nonce: *nonce,
                        hash: *hash,
                    })
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

    fn recompute_queue_counts(&mut self) {
        self.sender_queue_counts.clear();
        self.ready_total = 0;
        self.blocked_total = 0;
        let senders = self.sender_queues.keys().copied().collect::<Vec<_>>();
        for sender in senders {
            self.refresh_sender_counts(sender);
        }
    }

    fn sender_queue_positions(&self, sender: Address) -> Vec<SenderQueuePosition> {
        let Some(queue) = self.sender_queues.get(&sender) else {
            return Vec::new();
        };

        let mut positions = Vec::new();
        let mut next_executable_nonce = None;
        let mut gap_seen = false;

        for (nonce, hash) in queue {
            if !self.pending.contains_key(hash) {
                continue;
            }
            let state = queue_entry_state(&mut next_executable_nonce, &mut gap_seen, *nonce);
            positions.push(SenderQueuePosition {
                hash: *hash,
                nonce: *nonce,
                state,
            });
        }

        positions
    }

    fn queue_transitions(
        &self,
        sender: Address,
        previous_positions: &[SenderQueuePosition],
    ) -> Vec<SchedulerQueueTransition> {
        let previous_by_hash = previous_positions
            .iter()
            .map(|position| (position.hash, position.state))
            .collect::<BTreeMap<_, _>>();

        self.sender_queue_positions(sender)
            .into_iter()
            .filter_map(|position| match previous_by_hash.get(&position.hash) {
                Some(state) if *state == position.state => None,
                _ => Some(SchedulerQueueTransition {
                    hash: position.hash,
                    sender,
                    nonce: position.nonce,
                    state: position.state,
                }),
            })
            .collect()
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
            match queue_entry_state(&mut next_executable_nonce, &mut gap_seen, *nonce) {
                SchedulerQueueState::Ready => ready = ready.saturating_add(1),
                SchedulerQueueState::Blocked { .. } => blocked = blocked.saturating_add(1),
            }
        }

        SenderQueueCounts { ready, blocked }
    }
}

/// Advances the nonce-gap state machine for one queue entry and returns whether
/// the entry is Ready or Blocked. Called from every queue traversal to avoid
/// duplicating the identical three-arm match.
fn queue_entry_state(
    next_executable_nonce: &mut Option<u64>,
    gap_seen: &mut bool,
    nonce: u64,
) -> SchedulerQueueState {
    match *next_executable_nonce {
        None => {
            *next_executable_nonce = nonce.checked_add(1);
            SchedulerQueueState::Ready
        }
        Some(expected) if !*gap_seen && nonce == expected => {
            *next_executable_nonce = nonce.checked_add(1);
            SchedulerQueueState::Ready
        }
        Some(expected) => {
            *gap_seen = true;
            SchedulerQueueState::Blocked {
                expected_nonce: expected,
            }
        }
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

fn build_sender_queues_from_pending(
    pending: &BTreeMap<TxHash, ValidatedTransaction>,
) -> Result<BTreeMap<Address, BTreeMap<u64, TxHash>>, SchedulerSnapshotError> {
    let mut sender_queues = BTreeMap::new();
    for tx in pending.values() {
        insert_sender_queue_entry(
            &mut sender_queues,
            tx.decoded.sender,
            tx.decoded.nonce,
            tx.hash(),
        )?;
    }
    Ok(sender_queues)
}

fn build_sender_queues_from_snapshot(
    persisted_sender_queues: &[PersistedSenderQueueSnapshot],
    pending: &BTreeMap<TxHash, ValidatedTransaction>,
) -> Result<BTreeMap<Address, BTreeMap<u64, TxHash>>, SchedulerSnapshotError> {
    let mut sender_queues = BTreeMap::new();
    for persisted_queue in persisted_sender_queues {
        for entry in &persisted_queue.queued {
            let Some(tx) = pending.get(&entry.hash) else {
                return Err(SchedulerSnapshotError::MissingPendingTransaction { hash: entry.hash });
            };
            if tx.decoded.sender != persisted_queue.sender || tx.decoded.nonce != entry.nonce {
                return Err(SchedulerSnapshotError::QueueEntryMismatch { hash: entry.hash });
            }
            insert_sender_queue_entry(
                &mut sender_queues,
                persisted_queue.sender,
                entry.nonce,
                entry.hash,
            )?;
        }
    }
    Ok(sender_queues)
}

fn insert_sender_queue_entry(
    sender_queues: &mut BTreeMap<Address, BTreeMap<u64, TxHash>>,
    sender: Address,
    nonce: u64,
    hash: TxHash,
) -> Result<(), SchedulerSnapshotError> {
    let queue = sender_queues.entry(sender).or_default();
    if queue.insert(nonce, hash).is_some() {
        return Err(SchedulerSnapshotError::DuplicateSenderNonce { sender, nonce });
    }
    Ok(())
}
