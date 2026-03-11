//! In-memory state machines used by replay to reconstruct mempool and candidate lifecycles.

use ahash::RandomState;
use common::{Address, BlockHash, TxHash};
use event_log::{EventEnvelope, EventPayload, TxBlocked, TxDecoded, TxDropped, TxReady, TxReorged};
use hashbrown::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

type FastMap<K, V> = HashMap<K, V, RandomState>;
type FastSet<T> = HashSet<T, RandomState>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Replay-time queue state for a sender/nonce position.
pub enum ReplayQueueState {
    Ready,
    Blocked { expected_nonce: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Queue entry emitted in replay snapshots.
pub struct ReplaySenderQueueEntry {
    pub hash: TxHash,
    pub nonce: u64,
    pub state: ReplayQueueState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Replay snapshot of one sender queue.
pub struct ReplaySenderQueue {
    pub sender: Address,
    pub queued: Vec<ReplaySenderQueueEntry>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
/// Accumulated lifecycle data for one candidate across queueing, simulation, and assembly.
pub struct CandidateLifecycleEntry {
    #[serde(default)]
    pub tx_hash: TxHash,
    #[serde(default)]
    pub member_tx_hashes: Vec<TxHash>,
    #[serde(default)]
    pub chain_id: Option<u64>,
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub score: u32,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub feature_engine_version: String,
    #[serde(default)]
    pub scorer_version: String,
    #[serde(default)]
    pub strategy_version: String,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub detected_unix_ms: i64,
    #[serde(default)]
    pub simulation_status: Option<String>,
    #[serde(default)]
    pub assembly_status: Option<String>,
    #[serde(default)]
    pub simulation_block_number: Option<u64>,
    #[serde(default)]
    pub assembly_block_number: Option<u64>,
    #[serde(default)]
    pub assembly_reason: Option<String>,
    #[serde(default)]
    pub replaced_candidate_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
/// Candidate lifecycle snapshot captured at a particular sequence id.
pub struct CandidateLifecycleSnapshot {
    pub seq_id: u64,
    #[serde(default)]
    pub candidates: BTreeMap<String, CandidateLifecycleEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Current replayed lifecycle status for a transaction hash.
pub enum TxLifecycleStatus {
    Pending,
    Replaced {
        by: TxHash,
    },
    Dropped {
        reason: String,
    },
    ConfirmedProvisional {
        block_number: u64,
        block_hash: BlockHash,
    },
    ConfirmedFinal {
        block_number: u64,
        block_hash: BlockHash,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State change emitted while applying one event to the replay model.
pub enum StateTransition {
    Pending {
        hash: TxHash,
    },
    Replaced {
        old_hash: TxHash,
        new_hash: TxHash,
    },
    Dropped {
        hash: TxHash,
        reason: String,
    },
    ConfirmedProvisional {
        hash: TxHash,
        block_hash: BlockHash,
    },
    ConfirmedFinal {
        hash: TxHash,
        block_hash: BlockHash,
    },
    ReorgReopened {
        hash: TxHash,
        old_block_hash: BlockHash,
        new_block_hash: BlockHash,
    },
    QueueReady {
        hash: TxHash,
        sender: Address,
        nonce: u64,
    },
    QueueBlocked {
        hash: TxHash,
        sender: Address,
        nonce: u64,
        expected_nonce: Option<u64>,
    },
}

#[derive(Clone, Debug)]
struct TxEntry {
    sender: Option<Address>,
    nonce: Option<u64>,
    status: TxLifecycleStatus,
    queue_state: Option<ReplayQueueState>,
}

#[derive(Clone, Debug, Default)]
/// In-memory transaction lifecycle model reconstructed from event-log payloads.
pub struct MempoolState {
    txs: FastMap<TxHash, TxEntry>,
    sender_queues: BTreeMap<Address, BTreeMap<u64, TxHash>>,
    block_confirmations: FastMap<BlockHash, FastSet<TxHash>>,
}

#[derive(Clone, Debug, Default)]
/// Tracks lifecycle details for searcher candidates across downstream stages.
pub struct CandidateLifecycleState {
    candidates: BTreeMap<String, CandidateLifecycleEntry>,
    tx_candidates: BTreeMap<TxHash, BTreeSet<String>>,
}

impl MempoolState {
    /// Seeds replay state from a set of hashes that are already known to be pending.
    pub fn from_pending_hashes(pending_hashes: &[TxHash]) -> Self {
        let mut state = Self::default();
        for hash in pending_hashes {
            state.txs.insert(
                *hash,
                TxEntry {
                    sender: None,
                    nonce: None,
                    status: TxLifecycleStatus::Pending,
                    queue_state: None,
                },
            );
        }
        state
    }

    /// Seeds replay state from a checkpoint that already includes sender queue reconstruction.
    pub fn from_checkpoint(pending_hashes: &[TxHash], sender_queues: &[ReplaySenderQueue]) -> Self {
        let mut state = Self::from_pending_hashes(pending_hashes);
        for queue in sender_queues {
            for entry in &queue.queued {
                state.txs.entry(entry.hash).or_insert(TxEntry {
                    sender: Some(queue.sender),
                    nonce: Some(entry.nonce),
                    status: TxLifecycleStatus::Pending,
                    queue_state: Some(entry.state),
                });
                state
                    .sender_queues
                    .entry(queue.sender)
                    .or_default()
                    .insert(entry.nonce, entry.hash);
            }
        }
        state
    }

    /// Applies one event and returns the resulting transaction state transitions.
    pub fn apply_event(&mut self, event: &EventEnvelope) -> Vec<StateTransition> {
        match &event.payload {
            EventPayload::TxDecoded(decoded) => self.apply_decoded(decoded),
            EventPayload::TxDropped(dropped) => self.apply_dropped(dropped),
            EventPayload::TxReady(ready) => self.apply_ready(ready),
            EventPayload::TxBlocked(blocked) => self.apply_blocked(blocked),
            EventPayload::TxConfirmedProvisional(confirmed) => {
                let (sender, nonce) = self
                    .txs
                    .get(&confirmed.hash)
                    .map(|entry| (entry.sender, entry.nonce))
                    .unwrap_or((None, None));
                let entry = self.txs.entry(confirmed.hash).or_insert(TxEntry {
                    sender: None,
                    nonce: None,
                    status: TxLifecycleStatus::Pending,
                    queue_state: None,
                });
                entry.status = TxLifecycleStatus::ConfirmedProvisional {
                    block_number: confirmed.block_number,
                    block_hash: confirmed.block_hash,
                };
                entry.queue_state = None;
                if let (Some(sender), Some(nonce)) = (sender, nonce) {
                    self.remove_sender_queue_entry(confirmed.hash, sender, nonce);
                    self.recompute_sender_queue_states(sender);
                }
                self.block_confirmations
                    .entry(confirmed.block_hash)
                    .or_default()
                    .insert(confirmed.hash);
                vec![StateTransition::ConfirmedProvisional {
                    hash: confirmed.hash,
                    block_hash: confirmed.block_hash,
                }]
            }
            EventPayload::TxConfirmedFinal(confirmed) => {
                let (sender, nonce) = self
                    .txs
                    .get(&confirmed.hash)
                    .map(|entry| (entry.sender, entry.nonce))
                    .unwrap_or((None, None));
                let entry = self.txs.entry(confirmed.hash).or_insert(TxEntry {
                    sender: None,
                    nonce: None,
                    status: TxLifecycleStatus::Pending,
                    queue_state: None,
                });
                entry.status = TxLifecycleStatus::ConfirmedFinal {
                    block_number: confirmed.block_number,
                    block_hash: confirmed.block_hash,
                };
                entry.queue_state = None;
                if let (Some(sender), Some(nonce)) = (sender, nonce) {
                    self.remove_sender_queue_entry(confirmed.hash, sender, nonce);
                    self.recompute_sender_queue_states(sender);
                }
                self.block_confirmations
                    .entry(confirmed.block_hash)
                    .or_default()
                    .insert(confirmed.hash);
                vec![StateTransition::ConfirmedFinal {
                    hash: confirmed.hash,
                    block_hash: confirmed.block_hash,
                }]
            }
            EventPayload::TxReorged(reorged) => self.apply_reorg(reorged),
            EventPayload::TxReplaced(replaced) => {
                let (sender, nonce) = self
                    .txs
                    .get(&replaced.hash)
                    .map(|entry| (entry.sender, entry.nonce))
                    .unwrap_or((None, None));
                if let Some(entry) = self.txs.get_mut(&replaced.hash) {
                    entry.status = TxLifecycleStatus::Replaced {
                        by: replaced.replaced_by,
                    };
                    entry.queue_state = None;
                }
                if let (Some(sender), Some(nonce)) = (sender, nonce) {
                    self.remove_sender_queue_entry(replaced.hash, sender, nonce);
                    self.recompute_sender_queue_states(sender);
                }
                vec![StateTransition::Replaced {
                    old_hash: replaced.hash,
                    new_hash: replaced.replaced_by,
                }]
            }
            EventPayload::TxSeen(_)
            | EventPayload::TxFetched(_)
            | EventPayload::CandidateQueued(_)
            | EventPayload::SimDispatched(_)
            | EventPayload::OppDetected(_)
            | EventPayload::SimCompleted(_)
            | EventPayload::AssemblyDecisionApplied(_)
            | EventPayload::BundleSubmitted(_) => Vec::new(),
        }
    }

    /// Returns the replayed lifecycle for a transaction hash.
    pub fn lifecycle(&self, hash: &TxHash) -> Option<&TxLifecycleStatus> {
        self.txs.get(hash).map(|entry| &entry.status)
    }

    /// Returns all hashes currently considered pending by the replay state.
    pub fn pending_hashes(&self) -> Vec<TxHash> {
        self.txs
            .iter()
            .filter_map(|(hash, entry)| match entry.status {
                TxLifecycleStatus::Pending => Some(*hash),
                _ => None,
            })
            .collect()
    }

    /// Reconstructs sender queues with ready/blocked classification for pending transactions.
    pub fn sender_queues(&self) -> Vec<ReplaySenderQueue> {
        self.sender_queues
            .iter()
            .filter_map(|(sender, queue)| {
                let mut next_executable_nonce = None;
                let mut gap_seen = false;
                let mut queued = Vec::new();

                for (nonce, hash) in queue {
                    let Some(entry) = self.txs.get(hash) else {
                        continue;
                    };
                    if !matches!(entry.status, TxLifecycleStatus::Pending) {
                        continue;
                    }

                    let computed_state = match next_executable_nonce {
                        None => {
                            next_executable_nonce = nonce.checked_add(1);
                            ReplayQueueState::Ready
                        }
                        Some(expected) if !gap_seen && *nonce == expected => {
                            next_executable_nonce = nonce.checked_add(1);
                            ReplayQueueState::Ready
                        }
                        Some(expected) => {
                            gap_seen = true;
                            ReplayQueueState::Blocked {
                                expected_nonce: expected,
                            }
                        }
                    };

                    queued.push(ReplaySenderQueueEntry {
                        hash: *hash,
                        nonce: *nonce,
                        state: entry.queue_state.unwrap_or(computed_state),
                    });
                }

                if queued.is_empty() {
                    None
                } else {
                    Some(ReplaySenderQueue {
                        sender: *sender,
                        queued,
                    })
                }
            })
            .collect()
    }

    fn apply_decoded(&mut self, decoded: &TxDecoded) -> Vec<StateTransition> {
        let mut transitions = Vec::new();
        if let Some(old_hash) = self
            .sender_queues
            .get(&decoded.sender)
            .and_then(|queue| queue.get(&decoded.nonce).copied())
            && old_hash != decoded.hash
        {
            if let Some(old_entry) = self.txs.get_mut(&old_hash) {
                old_entry.status = TxLifecycleStatus::Replaced { by: decoded.hash };
                old_entry.queue_state = None;
            }
            transitions.push(StateTransition::Replaced {
                old_hash,
                new_hash: decoded.hash,
            });
        }

        self.ensure_sender_queue_entry(decoded.hash, decoded.sender, decoded.nonce);
        self.recompute_sender_queue_states(decoded.sender);
        transitions.push(StateTransition::Pending { hash: decoded.hash });

        transitions
    }

    fn apply_dropped(&mut self, dropped: &TxDropped) -> Vec<StateTransition> {
        let (sender, nonce) = self
            .txs
            .get(&dropped.hash)
            .map(|entry| (entry.sender, entry.nonce))
            .unwrap_or((None, None));
        let entry = self.txs.entry(dropped.hash).or_insert(TxEntry {
            sender: None,
            nonce: None,
            status: TxLifecycleStatus::Dropped {
                reason: dropped.reason.clone(),
            },
            queue_state: None,
        });
        entry.status = TxLifecycleStatus::Dropped {
            reason: dropped.reason.clone(),
        };
        entry.queue_state = None;
        if let (Some(sender), Some(nonce)) = (sender, nonce) {
            self.remove_sender_queue_entry(dropped.hash, sender, nonce);
            self.recompute_sender_queue_states(sender);
        }
        vec![StateTransition::Dropped {
            hash: dropped.hash,
            reason: dropped.reason.clone(),
        }]
    }

    fn apply_reorg(&mut self, reorged: &TxReorged) -> Vec<StateTransition> {
        let mut transitions = Vec::new();
        let Some((should_reopen, sender, nonce)) = self.txs.get(&reorged.hash).map(|entry| {
            (
                matches!(
                    entry.status,
                    TxLifecycleStatus::ConfirmedProvisional { block_hash, .. }
                        | TxLifecycleStatus::ConfirmedFinal { block_hash, .. }
                        if block_hash == reorged.old_block_hash
                ),
                entry.sender,
                entry.nonce,
            )
        }) else {
            return transitions;
        };

        if should_reopen {
            if let Some(entry) = self.txs.get_mut(&reorged.hash) {
                entry.status = TxLifecycleStatus::Pending;
            }
            if let (Some(sender), Some(nonce)) = (sender, nonce) {
                self.ensure_sender_queue_entry(reorged.hash, sender, nonce);
                self.recompute_sender_queue_states(sender);
            }
            if let Some(set) = self.block_confirmations.get_mut(&reorged.old_block_hash) {
                set.remove(&reorged.hash);
            }
            transitions.push(StateTransition::ReorgReopened {
                hash: reorged.hash,
                old_block_hash: reorged.old_block_hash,
                new_block_hash: reorged.new_block_hash,
            });
        }
        transitions
    }

    fn apply_ready(&mut self, ready: &TxReady) -> Vec<StateTransition> {
        self.ensure_sender_queue_entry(ready.hash, ready.sender, ready.nonce);
        self.recompute_sender_queue_states(ready.sender);
        if let Some(entry) = self.txs.get_mut(&ready.hash) {
            entry.queue_state = Some(ReplayQueueState::Ready);
        }
        vec![StateTransition::QueueReady {
            hash: ready.hash,
            sender: ready.sender,
            nonce: ready.nonce,
        }]
    }

    fn apply_blocked(&mut self, blocked: &TxBlocked) -> Vec<StateTransition> {
        self.ensure_sender_queue_entry(blocked.hash, blocked.sender, blocked.nonce);
        self.recompute_sender_queue_states(blocked.sender);
        if let Some(expected_nonce) = blocked.expected_nonce
            && let Some(entry) = self.txs.get_mut(&blocked.hash)
        {
            entry.queue_state = Some(ReplayQueueState::Blocked { expected_nonce });
        }
        vec![StateTransition::QueueBlocked {
            hash: blocked.hash,
            sender: blocked.sender,
            nonce: blocked.nonce,
            expected_nonce: blocked.expected_nonce,
        }]
    }

    fn ensure_sender_queue_entry(&mut self, hash: TxHash, sender: Address, nonce: u64) {
        let entry = self.txs.entry(hash).or_insert(TxEntry {
            sender: Some(sender),
            nonce: Some(nonce),
            status: TxLifecycleStatus::Pending,
            queue_state: None,
        });
        entry.sender = Some(sender);
        entry.nonce = Some(nonce);
        entry.status = TxLifecycleStatus::Pending;
        self.sender_queues
            .entry(sender)
            .or_default()
            .insert(nonce, hash);
    }

    fn remove_sender_queue_entry(&mut self, hash: TxHash, sender: Address, nonce: u64) {
        let mut remove_sender = false;
        if let Some(queue) = self.sender_queues.get_mut(&sender) {
            if queue
                .get(&nonce)
                .is_some_and(|queued_hash| *queued_hash == hash)
            {
                queue.remove(&nonce);
            }
            remove_sender = queue.is_empty();
        }
        if remove_sender {
            self.sender_queues.remove(&sender);
        }
    }

    fn recompute_sender_queue_states(&mut self, sender: Address) {
        let Some(queue_entries) = self.sender_queues.get(&sender).map(|queue| {
            queue
                .iter()
                .map(|(nonce, hash)| (*nonce, *hash))
                .collect::<Vec<_>>()
        }) else {
            return;
        };

        for (_, hash) in &queue_entries {
            if let Some(entry) = self.txs.get_mut(hash) {
                entry.queue_state = None;
            }
        }

        let mut next_executable_nonce = None;
        let mut gap_seen = false;
        let mut stale_nonces = Vec::new();
        let mut updates = Vec::new();

        for (nonce, hash) in &queue_entries {
            let Some(entry) = self.txs.get(hash) else {
                stale_nonces.push(*nonce);
                continue;
            };
            if !matches!(entry.status, TxLifecycleStatus::Pending) {
                stale_nonces.push(*nonce);
                continue;
            }

            let state = match next_executable_nonce {
                None => {
                    next_executable_nonce = nonce.checked_add(1);
                    ReplayQueueState::Ready
                }
                Some(expected) if !gap_seen && *nonce == expected => {
                    next_executable_nonce = nonce.checked_add(1);
                    ReplayQueueState::Ready
                }
                Some(expected) => {
                    gap_seen = true;
                    ReplayQueueState::Blocked {
                        expected_nonce: expected,
                    }
                }
            };
            updates.push((*hash, state));
        }

        let mut remove_sender = false;
        if let Some(queue) = self.sender_queues.get_mut(&sender) {
            for nonce in stale_nonces {
                queue.remove(&nonce);
            }
            remove_sender = queue.is_empty();
        }
        if remove_sender {
            self.sender_queues.remove(&sender);
        }

        for (hash, state) in updates {
            if let Some(entry) = self.txs.get_mut(&hash) {
                entry.queue_state = Some(state);
            }
        }
    }
}

impl CandidateLifecycleState {
    /// Applies one event to the candidate lifecycle model.
    pub fn apply_event(&mut self, event: &EventEnvelope) {
        match &event.payload {
            EventPayload::CandidateQueued(queued) => {
                self.index_candidate(&queued.candidate_id, queued.tx_hash);
                let entry = self
                    .candidates
                    .entry(queued.candidate_id.clone())
                    .or_default();
                entry.tx_hash = queued.tx_hash;
                entry.member_tx_hashes = queued.member_tx_hashes.clone();
                entry.chain_id = queued.chain_id;
                entry.strategy = queued.strategy.clone();
                entry.score = queued.score;
                entry.protocol = queued.protocol.clone();
                entry.category = queued.category.clone();
                entry.feature_engine_version = queued.feature_engine_version.clone();
                entry.scorer_version = queued.scorer_version.clone();
                entry.strategy_version = queued.strategy_version.clone();
                entry.reasons = queued.reasons.clone();
                entry.detected_unix_ms = queued.detected_unix_ms;
            }
            EventPayload::SimDispatched(dispatched) => {
                self.index_candidate(&dispatched.candidate_id, dispatched.tx_hash);
                let entry = self
                    .candidates
                    .entry(dispatched.candidate_id.clone())
                    .or_default();
                entry.tx_hash = dispatched.tx_hash;
                if entry.member_tx_hashes.is_empty() {
                    entry.member_tx_hashes = dispatched.member_tx_hashes.clone();
                }
                entry.simulation_block_number = Some(dispatched.block_number);
            }
            EventPayload::SimCompleted(completed) => {
                let candidate_ids = self
                    .tx_candidates
                    .get(&completed.hash)
                    .cloned()
                    .unwrap_or_default();
                for candidate_id in candidate_ids {
                    let entry = self.candidates.entry(candidate_id).or_default();
                    entry.tx_hash = completed.hash;
                    entry.simulation_status = Some(completed.status.clone());
                }
            }
            EventPayload::AssemblyDecisionApplied(applied) => {
                self.index_candidate(&applied.candidate_id, applied.tx_hash);
                let entry = self
                    .candidates
                    .entry(applied.candidate_id.clone())
                    .or_default();
                entry.tx_hash = applied.tx_hash;
                entry.assembly_status = Some(applied.decision.clone());
                entry.assembly_block_number = Some(applied.block_number);
                entry.assembly_reason = applied.reason.clone();
                entry.replaced_candidate_ids = applied.replaced_candidate_ids.clone();
            }
            _ => {}
        }
    }

    /// Captures the current candidate lifecycle snapshot at the provided sequence id.
    pub fn snapshot(&self, seq_id: u64) -> CandidateLifecycleSnapshot {
        CandidateLifecycleSnapshot {
            seq_id,
            candidates: self.candidates.clone(),
        }
    }

    fn index_candidate(&mut self, candidate_id: &str, tx_hash: TxHash) {
        self.tx_candidates
            .entry(tx_hash)
            .or_default()
            .insert(candidate_id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::SourceId;
    use event_log::{TxConfirmed, TxReplaced, TxSeen};

    fn hash(value: u8) -> TxHash {
        [value; 32]
    }

    fn address(value: u8) -> Address {
        [value; 20]
    }

    fn block(value: u8) -> BlockHash {
        [value; 32]
    }

    fn envelope(seq_id: u64, payload: EventPayload) -> EventEnvelope {
        EventEnvelope {
            seq_id,
            ingest_ts_unix_ms: 1_700_000_000_000 + seq_id as i64,
            ingest_ts_mono_ns: seq_id * 10,
            source_id: SourceId::new("test-source"),
            payload,
        }
    }

    fn decoded(hash_value: u8, sender_value: u8, nonce: u64) -> EventPayload {
        EventPayload::TxDecoded(TxDecoded {
            hash: hash(hash_value),
            tx_type: 2,
            sender: address(sender_value),
            nonce,
            chain_id: Some(1),
            to: None,
            value_wei: None,
            gas_limit: None,
            gas_price_wei: None,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            max_fee_per_blob_gas_wei: None,
            calldata_len: None,
        })
    }

    #[test]
    fn detects_replacement_by_sender_nonce() {
        let mut state = MempoolState::default();

        state.apply_event(&envelope(1, decoded(1, 10, 7)));
        let transitions = state.apply_event(&envelope(2, decoded(2, 10, 7)));

        assert!(transitions.contains(&StateTransition::Replaced {
            old_hash: hash(1),
            new_hash: hash(2),
        }));
        assert_eq!(
            state.lifecycle(&hash(1)),
            Some(&TxLifecycleStatus::Replaced { by: hash(2) })
        );
        assert_eq!(state.lifecycle(&hash(2)), Some(&TxLifecycleStatus::Pending));
    }

    #[test]
    fn reorg_rolls_back_confirmed_to_pending() {
        let mut state = MempoolState::default();

        state.apply_event(&envelope(1, decoded(3, 9, 1)));
        state.apply_event(&envelope(
            2,
            EventPayload::TxConfirmedProvisional(TxConfirmed {
                hash: hash(3),
                block_number: 1_000,
                block_hash: block(5),
            }),
        ));
        let transitions = state.apply_event(&envelope(
            3,
            EventPayload::TxReorged(TxReorged {
                hash: hash(3),
                old_block_hash: block(5),
                new_block_hash: block(6),
            }),
        ));

        assert_eq!(state.lifecycle(&hash(3)), Some(&TxLifecycleStatus::Pending));
        assert_eq!(
            transitions,
            vec![StateTransition::ReorgReopened {
                hash: hash(3),
                old_block_hash: block(5),
                new_block_hash: block(6),
            }]
        );
    }

    #[test]
    fn dropped_tx_removes_sender_queue_entry() {
        let mut state = MempoolState::default();

        state.apply_event(&envelope(1, decoded(7, 2, 11)));
        state.apply_event(&envelope(
            2,
            EventPayload::TxDropped(TxDropped {
                hash: hash(7),
                reason: "evicted".to_owned(),
            }),
        ));
        let transitions = state.apply_event(&envelope(3, decoded(8, 2, 11)));

        assert_eq!(state.lifecycle(&hash(8)), Some(&TxLifecycleStatus::Pending));
        assert!(!transitions.contains(&StateTransition::Replaced {
            old_hash: hash(7),
            new_hash: hash(8),
        }));
    }

    #[test]
    fn dropped_tx_without_prior_decode_still_records_lifecycle() {
        let mut state = MempoolState::default();

        let transitions = state.apply_event(&envelope(
            1,
            EventPayload::TxDropped(TxDropped {
                hash: hash(12),
                reason: "queue_full".to_owned(),
            }),
        ));

        assert_eq!(
            transitions,
            vec![StateTransition::Dropped {
                hash: hash(12),
                reason: "queue_full".to_owned(),
            }]
        );
        assert_eq!(
            state.lifecycle(&hash(12)),
            Some(&TxLifecycleStatus::Dropped {
                reason: "queue_full".to_owned(),
            })
        );
        assert!(state.pending_hashes().is_empty());
    }

    #[test]
    fn ignores_unrelated_payload_without_panicking() {
        let mut state = MempoolState::default();
        let transitions = state.apply_event(&envelope(
            1,
            EventPayload::TxSeen(TxSeen {
                hash: hash(1),
                peer_id: "peer-a".to_owned(),
                seen_at_unix_ms: 1_700_000_000_000,
                seen_at_mono_ns: 10,
            }),
        ));
        assert!(transitions.is_empty());
    }

    #[test]
    fn explicit_replaced_payload_sets_lifecycle() {
        let mut state = MempoolState::default();
        state.apply_event(&envelope(1, decoded(9, 4, 1)));
        state.apply_event(&envelope(
            2,
            EventPayload::TxReplaced(TxReplaced {
                hash: hash(9),
                replaced_by: hash(10),
            }),
        ));
        assert_eq!(
            state.lifecycle(&hash(9)),
            Some(&TxLifecycleStatus::Replaced { by: hash(10) })
        );
    }
}
