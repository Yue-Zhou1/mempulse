mod mempool_state;

use common::TxHash;
use event_log::{EventEnvelope, EventPayload, sort_deterministic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use mempool_state::{MempoolState, StateTransition, TxLifecycleStatus};

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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LifecycleCheckpoint {
    pub seq_id: u64,
    pub pending_hashes: Vec<TxHash>,
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
            });
        }
    }

    out
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
            });
        }
    }

    frames
}

fn replay_snapshot(events: &[EventEnvelope], stride: usize) -> Vec<ReplayFrame> {
    let mut sorted = events.to_vec();
    sort_deterministic(&mut sorted);

    let mut pending = BTreeSet::new();
    let mut frames = Vec::new();
    let mut count = 0usize;

    for event in &sorted {
        count += 1;
        match &event.payload {
            EventPayload::TxDecoded(decoded) => {
                pending.insert(decoded.hash);
            }
            EventPayload::TxDropped(dropped) => {
                pending.remove(&dropped.hash);
            }
            EventPayload::TxConfirmedFinal(confirmed) => {
                pending.remove(&confirmed.hash);
            }
            EventPayload::TxReplaced(replaced) => {
                pending.remove(&replaced.hash);
                pending.insert(replaced.replaced_by);
            }
            _ => {}
        }

        if count % stride == 0 || count == sorted.len() {
            frames.push(ReplayFrame {
                seq_hi: event.seq_id,
                timestamp_unix_ms: event.ingest_ts_unix_ms,
                pending: pending.iter().copied().collect(),
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
    use event_log::{TxConfirmed, TxDecoded, TxDropped};

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
}
