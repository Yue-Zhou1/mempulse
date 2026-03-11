#![forbid(unsafe_code)]

mod mempool_state;

use common::TxHash;
#[cfg(test)]
use event_log::EventPayload;
use event_log::{EventEnvelope, sort_deterministic};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::sync::Arc;

pub use mempool_state::{
    CandidateLifecycleEntry, CandidateLifecycleSnapshot, CandidateLifecycleState, MempoolState,
    ReplayQueueState, ReplaySenderQueue, ReplaySenderQueueEntry, StateTransition,
    TxLifecycleStatus,
};
pub use sim_engine::{
    ChainContext as SimulationChainContext, SimulationBatchResult, TxSimulationResult,
};

type SharedError = Arc<dyn StdError + Send + Sync>;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("checkpoint mismatch at seq {seq}: expected {expected:?}, got {actual:?}")]
    CheckpointMismatch {
        seq: u64,
        expected: Box<[u8; 32]>,
        actual: Box<[u8; 32]>,
    },
    #[error(transparent)]
    Other(SharedError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayMode {
    DeterministicEventReplay,
    SnapshotReplay,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayFrame {
    pub seq_hi: u64,
    pub timestamp_unix_ms: i64,
    pub pending: Vec<TxHash>,
    #[serde(default)]
    pub sender_queues: Vec<ReplaySenderQueue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LifecycleCheckpoint {
    pub seq_id: u64,
    pub pending_hashes: Vec<TxHash>,
    #[serde(default)]
    pub sender_queues: Vec<ReplaySenderQueue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckpointHash {
    pub seq_id: u64,
    pub pending_count: usize,
    pub checkpoint_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayDiffSummary {
    pub from_seq_id: u64,
    pub to_seq_id: u64,
    pub from_pending_count: usize,
    pub to_pending_count: usize,
    pub from_checkpoint_hash: [u8; 32],
    pub to_checkpoint_hash: [u8; 32],
    pub added_pending: Vec<TxHash>,
    pub removed_pending: Vec<TxHash>,
}

pub fn replay_frames(
    events: &[EventEnvelope],
    mode: ReplayMode,
    stride: usize,
) -> Vec<ReplayFrame> {
    match mode {
        ReplayMode::DeterministicEventReplay => replay_deterministic(events, stride.max(1)),
        ReplayMode::SnapshotReplay => replay_snapshot(events, stride.max(1)),
    }
}

pub fn lifecycle_checkpoints(
    events: &[EventEnvelope],
    checkpoint_seq_ids: &[u64],
) -> Vec<LifecycleCheckpoint> {
    let checkpoints: BTreeSet<u64> = checkpoint_seq_ids.iter().copied().collect();
    let mut sorted = events.to_vec();
    sort_deterministic(&mut sorted);

    let mut state = MempoolState::default();
    let mut out = Vec::new();

    for event in sorted {
        state.apply_event(&event);
        if checkpoints.contains(&event.seq_id) {
            let mut pending = state.pending_hashes();
            pending.sort_unstable();
            out.push(LifecycleCheckpoint {
                seq_id: event.seq_id,
                pending_hashes: pending,
                sender_queues: state.sender_queues(),
            });
        }
    }

    out
}

pub fn lifecycle_snapshot(
    events: &[EventEnvelope],
    checkpoint_seq_id: u64,
) -> Option<LifecycleCheckpoint> {
    lifecycle_checkpoints(events, &[checkpoint_seq_id])
        .into_iter()
        .next()
}

#[must_use]
pub fn lifecycle_checkpoint_hash(checkpoint: &LifecycleCheckpoint) -> [u8; 32] {
    let mut pending_hashes = checkpoint.pending_hashes.clone();
    pending_hashes.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(checkpoint.seq_id.to_le_bytes());
    hasher.update((pending_hashes.len() as u64).to_le_bytes());
    for hash in pending_hashes {
        hasher.update(hash);
    }
    hash_sender_queues(&mut hasher, &checkpoint.sender_queues);
    let digest = hasher.finalize();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn verify_lifecycle_checkpoint_hash(
    checkpoint: &LifecycleCheckpoint,
    expected: [u8; 32],
) -> std::result::Result<[u8; 32], ReplayError> {
    let actual = lifecycle_checkpoint_hash(checkpoint);
    if actual != expected {
        return Err(ReplayError::CheckpointMismatch {
            seq: checkpoint.seq_id,
            expected: Box::new(expected),
            actual: Box::new(actual),
        });
    }
    Ok(actual)
}

pub fn replay_from_checkpoint(
    events: &[EventEnvelope],
    checkpoint: &LifecycleCheckpoint,
    stride: usize,
) -> Vec<ReplayFrame> {
    replay_deterministic_from_checkpoint(events, checkpoint, stride.max(1))
}

#[must_use = "diff summaries must be inspected by callers"]
pub fn replay_diff_summary(
    events: &[EventEnvelope],
    from_seq_id: u64,
    to_seq_id: u64,
) -> Option<ReplayDiffSummary> {
    let (from_seq_id, to_seq_id) = if from_seq_id <= to_seq_id {
        (from_seq_id, to_seq_id)
    } else {
        (to_seq_id, from_seq_id)
    };
    let from_checkpoint = lifecycle_snapshot(events, from_seq_id)?;
    let to_checkpoint = lifecycle_snapshot(events, to_seq_id)?;

    let from_set: BTreeSet<TxHash> = from_checkpoint.pending_hashes.iter().copied().collect();
    let to_set: BTreeSet<TxHash> = to_checkpoint.pending_hashes.iter().copied().collect();

    let added_pending = to_set.difference(&from_set).copied().collect::<Vec<_>>();
    let removed_pending = from_set.difference(&to_set).copied().collect::<Vec<_>>();

    Some(ReplayDiffSummary {
        from_seq_id,
        to_seq_id,
        from_pending_count: from_checkpoint.pending_hashes.len(),
        to_pending_count: to_checkpoint.pending_hashes.len(),
        from_checkpoint_hash: lifecycle_checkpoint_hash(&from_checkpoint),
        to_checkpoint_hash: lifecycle_checkpoint_hash(&to_checkpoint),
        added_pending,
        removed_pending,
    })
}

fn replay_deterministic(events: &[EventEnvelope], stride: usize) -> Vec<ReplayFrame> {
    let mut sorted = events.to_vec();
    sort_deterministic(&mut sorted);

    let mut state = MempoolState::default();
    let mut frames = Vec::new();

    for (idx, event) in sorted.iter().enumerate() {
        state.apply_event(event);
        let should_emit = (idx + 1) % stride == 0 || idx + 1 == sorted.len();
        if should_emit {
            let mut pending = state.pending_hashes();
            pending.sort_unstable();
            frames.push(ReplayFrame {
                seq_hi: event.seq_id,
                timestamp_unix_ms: event.ingest_ts_unix_ms,
                pending,
                sender_queues: state.sender_queues(),
            });
        }
    }

    frames
}

fn replay_deterministic_from_checkpoint(
    events: &[EventEnvelope],
    checkpoint: &LifecycleCheckpoint,
    stride: usize,
) -> Vec<ReplayFrame> {
    let mut sorted = events.to_vec();
    sort_deterministic(&mut sorted);
    let sorted_tail = sorted
        .into_iter()
        .filter(|event| event.seq_id > checkpoint.seq_id)
        .collect::<Vec<_>>();

    if sorted_tail.is_empty() {
        return Vec::new();
    }

    let mut state = if checkpoint.sender_queues.is_empty() {
        MempoolState::from_pending_hashes(&checkpoint.pending_hashes)
    } else {
        MempoolState::from_checkpoint(&checkpoint.pending_hashes, &checkpoint.sender_queues)
    };
    let mut frames = Vec::new();
    for (idx, event) in sorted_tail.iter().enumerate() {
        state.apply_event(event);
        let should_emit = (idx + 1) % stride == 0 || idx + 1 == sorted_tail.len();
        if should_emit {
            let mut pending = state.pending_hashes();
            pending.sort_unstable();
            frames.push(ReplayFrame {
                seq_hi: event.seq_id,
                timestamp_unix_ms: event.ingest_ts_unix_ms,
                pending,
                sender_queues: state.sender_queues(),
            });
        }
    }

    frames
}

fn replay_snapshot(events: &[EventEnvelope], stride: usize) -> Vec<ReplayFrame> {
    let mut sorted = events.to_vec();
    sort_deterministic(&mut sorted);

    let mut state = MempoolState::default();
    let mut frames = Vec::new();
    let mut count = 0usize;

    for event in &sorted {
        count += 1;
        state.apply_event(event);

        if count.is_multiple_of(stride) || count == sorted.len() {
            let mut pending = state.pending_hashes();
            pending.sort_unstable();
            frames.push(ReplayFrame {
                seq_hi: event.seq_id,
                timestamp_unix_ms: event.ingest_ts_unix_ms,
                pending,
                sender_queues: state.sender_queues(),
            });
        }
    }

    frames
}

pub fn lifecycle_parity(
    events: &[EventEnvelope],
    checkpoint_seq_ids: &[u64],
) -> BTreeMap<u64, bool> {
    let expected = lifecycle_checkpoints(events, checkpoint_seq_ids);
    let deterministic = replay_frames(events, ReplayMode::DeterministicEventReplay, 1);
    let deterministic_by_seq: BTreeMap<u64, Vec<TxHash>> = deterministic
        .into_iter()
        .map(|frame| (frame.seq_hi, frame.pending))
        .collect();

    expected
        .into_iter()
        .map(|checkpoint| {
            let actual = deterministic_by_seq
                .get(&checkpoint.seq_id)
                .cloned()
                .unwrap_or_default();
            (checkpoint.seq_id, actual == checkpoint.pending_hashes)
        })
        .collect()
}

pub fn deterministic_checkpoint_hashes(
    events: &[EventEnvelope],
    stride: usize,
) -> Vec<CheckpointHash> {
    replay_frames(events, ReplayMode::DeterministicEventReplay, stride.max(1))
        .into_iter()
        .map(|frame| CheckpointHash {
            seq_id: frame.seq_hi,
            pending_count: frame.pending.len(),
            checkpoint_hash: frame_checkpoint_hash(&frame),
        })
        .collect()
}

pub fn checkpoint_hash_parity_percent(
    reference_events: &[EventEnvelope],
    candidate_events: &[EventEnvelope],
    stride: usize,
) -> f64 {
    let reference = deterministic_checkpoint_hashes(reference_events, stride.max(1));
    let candidate = deterministic_checkpoint_hashes(candidate_events, stride.max(1));
    if reference.is_empty() {
        return if candidate.is_empty() { 100.0 } else { 0.0 };
    }

    let candidate_by_seq: BTreeMap<u64, [u8; 32]> = candidate
        .into_iter()
        .map(|checkpoint| (checkpoint.seq_id, checkpoint.checkpoint_hash))
        .collect();

    let matched = reference
        .iter()
        .filter(|checkpoint| {
            candidate_by_seq
                .get(&checkpoint.seq_id)
                .is_some_and(|hash| *hash == checkpoint.checkpoint_hash)
        })
        .count();

    (matched as f64 / reference.len() as f64) * 100.0
}

fn frame_checkpoint_hash(frame: &ReplayFrame) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(frame.seq_hi.to_le_bytes());
    hasher.update(frame.timestamp_unix_ms.to_le_bytes());
    hasher.update((frame.pending.len() as u64).to_le_bytes());
    for hash in &frame.pending {
        hasher.update(hash);
    }
    hash_sender_queues(&mut hasher, &frame.sender_queues);
    let digest = hasher.finalize();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn hash_sender_queues(hasher: &mut Sha256, sender_queues: &[ReplaySenderQueue]) {
    hasher.update((sender_queues.len() as u64).to_le_bytes());
    for queue in sender_queues {
        hasher.update(queue.sender);
        hasher.update((queue.queued.len() as u64).to_le_bytes());
        for entry in &queue.queued {
            hasher.update(entry.hash);
            hasher.update(entry.nonce.to_le_bytes());
            match entry.state {
                ReplayQueueState::Ready => hasher.update([0]),
                ReplayQueueState::Blocked { expected_nonce } => {
                    hasher.update([1]);
                    hasher.update(expected_nonce.to_le_bytes());
                }
            }
        }
    }
}

pub fn current_lifecycle(events: &[EventEnvelope], hash: TxHash) -> Option<TxLifecycleStatus> {
    let mut sorted = events.to_vec();
    sort_deterministic(&mut sorted);
    let mut state = MempoolState::default();
    for event in &sorted {
        state.apply_event(event);
    }
    state.lifecycle(&hash).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{Address, BlockHash, SourceId};
    use event_log::{TxBlocked, TxConfirmed, TxDecoded, TxDropped, TxReady};

    fn hash(v: u8) -> TxHash {
        [v; 32]
    }
    fn address(v: u8) -> Address {
        [v; 20]
    }
    fn block(v: u8) -> BlockHash {
        [v; 32]
    }

    fn envelope(seq: u64, payload: EventPayload) -> EventEnvelope {
        EventEnvelope {
            seq_id: seq,
            ingest_ts_unix_ms: 1_700_000_000_000 + seq as i64,
            ingest_ts_mono_ns: seq * 100,
            source_id: SourceId::new("test"),
            payload,
        }
    }

    fn decoded(seq: u64, hash_v: u8, sender_v: u8, nonce: u64) -> EventEnvelope {
        envelope(
            seq,
            EventPayload::TxDecoded(TxDecoded {
                hash: hash(hash_v),
                tx_type: 2,
                sender: address(sender_v),
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
            }),
        )
    }

    fn ready(seq: u64, hash_v: u8, sender_v: u8, nonce: u64) -> EventEnvelope {
        envelope(
            seq,
            EventPayload::TxReady(TxReady {
                hash: hash(hash_v),
                sender: address(sender_v),
                nonce,
            }),
        )
    }

    fn blocked(
        seq: u64,
        hash_v: u8,
        sender_v: u8,
        nonce: u64,
        expected_nonce: u64,
    ) -> EventEnvelope {
        envelope(
            seq,
            EventPayload::TxBlocked(TxBlocked {
                hash: hash(hash_v),
                sender: address(sender_v),
                nonce,
                expected_nonce: Some(expected_nonce),
            }),
        )
    }

    #[test]
    fn deterministic_replay_is_stable_for_input_ordering() {
        let ordered = vec![
            decoded(1, 1, 9, 1),
            decoded(2, 2, 9, 1),
            envelope(
                3,
                EventPayload::TxDropped(TxDropped {
                    hash: hash(2),
                    reason: "evicted".to_owned(),
                }),
            ),
        ];
        let mut shuffled = vec![ordered[2].clone(), ordered[0].clone(), ordered[1].clone()];

        let frames_a = replay_frames(&ordered, ReplayMode::DeterministicEventReplay, 1);
        let frames_b = replay_frames(&shuffled, ReplayMode::DeterministicEventReplay, 1);

        assert_eq!(frames_a, frames_b);
        shuffled.reverse();
        let frames_c = replay_frames(&shuffled, ReplayMode::DeterministicEventReplay, 1);
        assert_eq!(frames_a, frames_c);
    }

    #[test]
    fn lifecycle_parity_matches_checkpoints() {
        let events = vec![
            decoded(1, 4, 1, 1),
            envelope(
                2,
                EventPayload::TxConfirmedFinal(TxConfirmed {
                    hash: hash(4),
                    block_number: 100,
                    block_hash: block(7),
                }),
            ),
            decoded(3, 5, 2, 1),
        ];

        let parity = lifecycle_parity(&events, &[1, 2, 3]);
        assert_eq!(parity.get(&1), Some(&true));
        assert_eq!(parity.get(&2), Some(&true));
        assert_eq!(parity.get(&3), Some(&true));
    }

    #[test]
    fn checkpoint_hash_parity_meets_slo_for_reordered_input() {
        let mut events = Vec::new();
        let mut seq = 1_u64;
        for idx in 0_u8..120_u8 {
            events.push(decoded(seq, idx.wrapping_add(1), idx, idx as u64));
            seq = seq.saturating_add(1);
            if idx % 3 == 0 {
                events.push(envelope(
                    seq,
                    EventPayload::TxDropped(TxDropped {
                        hash: hash(idx.wrapping_add(1)),
                        reason: "evicted".to_owned(),
                    }),
                ));
                seq = seq.saturating_add(1);
            }
        }
        let mut reordered = events.clone();
        reordered.reverse();

        let parity = crate::checkpoint_hash_parity_percent(&events, &reordered, 7);
        assert!(
            parity >= 99.99,
            "checkpoint hash parity below SLO: {parity:.4}%"
        );
    }

    #[test]
    fn lifecycle_snapshot_checkpoint_hash_is_stable_for_pending_ordering() {
        let checkpoint = LifecycleCheckpoint {
            seq_id: 10,
            pending_hashes: vec![hash(3), hash(1), hash(2)],
            sender_queues: Vec::new(),
        };
        let same_set_different_order = LifecycleCheckpoint {
            seq_id: 10,
            pending_hashes: vec![hash(2), hash(3), hash(1)],
            sender_queues: Vec::new(),
        };

        let hash_a = lifecycle_checkpoint_hash(&checkpoint);
        let hash_b = lifecycle_checkpoint_hash(&same_set_different_order);

        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn replay_from_checkpoint_matches_full_replay_tail() {
        let events = vec![
            decoded(1, 1, 1, 1),
            decoded(2, 2, 2, 1),
            envelope(
                3,
                EventPayload::TxDropped(TxDropped {
                    hash: hash(1),
                    reason: "evicted".to_owned(),
                }),
            ),
            decoded(4, 3, 3, 1),
            envelope(
                5,
                EventPayload::TxDropped(TxDropped {
                    hash: hash(2),
                    reason: "included".to_owned(),
                }),
            ),
        ];

        let checkpoint = lifecycle_snapshot(&events, 3).expect("checkpoint at seq=3");
        let full = replay_frames(&events, ReplayMode::DeterministicEventReplay, 1);
        let expected_tail = full
            .into_iter()
            .filter(|frame| frame.seq_hi > checkpoint.seq_id)
            .collect::<Vec<_>>();

        let from_checkpoint = replay_from_checkpoint(&events, &checkpoint, 1);
        assert_eq!(from_checkpoint, expected_tail);
    }

    #[test]
    fn replay_diff_summary_reports_added_and_removed_pending_hashes() {
        let events = vec![
            decoded(1, 1, 1, 1),
            decoded(2, 2, 2, 1),
            envelope(
                3,
                EventPayload::TxDropped(TxDropped {
                    hash: hash(1),
                    reason: "evicted".to_owned(),
                }),
            ),
            decoded(4, 3, 3, 1),
        ];

        let summary = replay_diff_summary(&events, 2, 4).expect("summary");
        assert_eq!(summary.from_seq_id, 2);
        assert_eq!(summary.to_seq_id, 4);
        assert_eq!(summary.from_pending_count, 2);
        assert_eq!(summary.to_pending_count, 2);
        assert_eq!(summary.added_pending, vec![hash(3)]);
        assert_eq!(summary.removed_pending, vec![hash(1)]);
        assert_ne!(summary.from_checkpoint_hash, summary.to_checkpoint_hash);
    }

    #[test]
    fn verify_lifecycle_checkpoint_hash_returns_typed_mismatch_error() {
        let checkpoint = LifecycleCheckpoint {
            seq_id: 7,
            pending_hashes: vec![hash(1), hash(2)],
            sender_queues: Vec::new(),
        };

        let err = verify_lifecycle_checkpoint_hash(&checkpoint, [0_u8; 32])
            .expect_err("checkpoint hash should mismatch");

        match err {
            ReplayError::CheckpointMismatch {
                seq,
                expected,
                actual,
            } => {
                assert_eq!(seq, 7);
                assert_eq!(*expected, [0_u8; 32]);
                assert_eq!(*actual, lifecycle_checkpoint_hash(&checkpoint));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn replay_reconstructs_sender_queue_state_from_queue_transition_events() {
        let events = vec![
            decoded(1, 7, 9, 7),
            ready(2, 7, 9, 7),
            decoded(3, 9, 9, 9),
            blocked(4, 9, 9, 9, 8),
            decoded(5, 8, 9, 8),
            ready(6, 8, 9, 8),
            ready(7, 9, 9, 9),
        ];

        let frames = replay_frames(&events, ReplayMode::DeterministicEventReplay, 1);
        assert_eq!(frames.len(), 7);

        assert_eq!(
            frames[3].sender_queues,
            vec![ReplaySenderQueue {
                sender: address(9),
                queued: vec![
                    ReplaySenderQueueEntry {
                        hash: hash(7),
                        nonce: 7,
                        state: ReplayQueueState::Ready,
                    },
                    ReplaySenderQueueEntry {
                        hash: hash(9),
                        nonce: 9,
                        state: ReplayQueueState::Blocked { expected_nonce: 8 },
                    },
                ],
            }]
        );
        assert_eq!(
            frames[6].sender_queues,
            vec![ReplaySenderQueue {
                sender: address(9),
                queued: vec![
                    ReplaySenderQueueEntry {
                        hash: hash(7),
                        nonce: 7,
                        state: ReplayQueueState::Ready,
                    },
                    ReplaySenderQueueEntry {
                        hash: hash(8),
                        nonce: 8,
                        state: ReplayQueueState::Ready,
                    },
                    ReplaySenderQueueEntry {
                        hash: hash(9),
                        nonce: 9,
                        state: ReplayQueueState::Ready,
                    },
                ],
            }]
        );

        let checkpoint = lifecycle_snapshot(&events, 4).expect("checkpoint at seq=4");
        let expected_tail = frames
            .iter()
            .filter(|frame| frame.seq_hi > checkpoint.seq_id)
            .cloned()
            .collect::<Vec<_>>();
        let from_checkpoint = replay_from_checkpoint(&events, &checkpoint, 1);

        assert_eq!(from_checkpoint, expected_tail);
    }
}
