//! Canonical event contracts used to move transaction lifecycle data between
//! ingest, runtime, storage, and replay components.

#![forbid(unsafe_code)]

use common::{Address, BlockHash, PeerId, SourceId, TxHash};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Top-level event envelope carrying ingest metadata and one typed payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub seq_id: u64,
    pub ingest_ts_unix_ms: i64,
    pub ingest_ts_mono_ns: u64,
    pub source_id: SourceId,
    pub payload: EventPayload,
}

impl EventEnvelope {
    /// Returns the deterministic ordering key used when events need a stable
    /// sort independent of original insertion order.
    pub fn order_key(&self) -> EventOrderKey {
        EventOrderKey {
            seq_id: self.seq_id,
            source_id: self.source_id.clone(),
            hash: self.payload.primary_hash(),
        }
    }
}

/// Monotonic sequencer for assigning globally ordered event ids.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalSequencer {
    next_seq_id: u64,
}

impl Default for GlobalSequencer {
    fn default() -> Self {
        Self { next_seq_id: 1 }
    }
}

impl GlobalSequencer {
    /// Creates a sequencer that continues after the supplied latest sequence id.
    pub fn from_latest_seq_id(latest_seq_id: Option<u64>) -> Self {
        let next_seq_id = latest_seq_id.unwrap_or(0).saturating_add(1).max(1);
        Self { next_seq_id }
    }

    /// Reserves and returns the next global sequence id.
    pub fn next_seq_id(&mut self) -> u64 {
        let seq_id = self.next_seq_id;
        self.next_seq_id = self.next_seq_id.saturating_add(1);
        seq_id
    }

    /// Assigns the next sequence id to an event envelope.
    pub fn assign(&mut self, mut event: EventEnvelope) -> EventEnvelope {
        event.seq_id = self.next_seq_id();
        event
    }
}

/// Stable sort key for event envelopes.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EventOrderKey {
    pub seq_id: u64,
    pub source_id: SourceId,
    pub hash: TxHash,
}

/// Canonical event payload variants emitted across the pipeline.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    TxSeen(TxSeen),
    TxFetched(TxFetched),
    TxDecoded(TxDecoded),
    TxReady(TxReady),
    TxBlocked(TxBlocked),
    CandidateQueued(CandidateQueued),
    SimDispatched(SimDispatched),
    OppDetected(OppDetected),
    SimCompleted(SimCompleted),
    AssemblyDecisionApplied(AssemblyDecisionApplied),
    BundleSubmitted(BundleSubmitted),
    TxReplaced(TxReplaced),
    TxDropped(TxDropped),
    TxConfirmedProvisional(TxConfirmed),
    TxConfirmedFinal(TxConfirmed),
    TxReorged(TxReorged),
}

impl EventPayload {
    /// Returns the primary transaction hash associated with the payload.
    pub fn primary_hash(&self) -> TxHash {
        match self {
            EventPayload::TxSeen(e) => e.hash,
            EventPayload::TxFetched(e) => e.hash,
            EventPayload::TxDecoded(e) => e.hash,
            EventPayload::TxReady(e) => e.hash,
            EventPayload::TxBlocked(e) => e.hash,
            EventPayload::CandidateQueued(e) => e.tx_hash,
            EventPayload::SimDispatched(e) => e.tx_hash,
            EventPayload::OppDetected(e) => e.hash,
            EventPayload::SimCompleted(e) => e.hash,
            EventPayload::AssemblyDecisionApplied(e) => e.tx_hash,
            EventPayload::BundleSubmitted(e) => e.hash,
            EventPayload::TxReplaced(e) => e.hash,
            EventPayload::TxDropped(e) => e.hash,
            EventPayload::TxConfirmedProvisional(e) => e.hash,
            EventPayload::TxConfirmedFinal(e) => e.hash,
            EventPayload::TxReorged(e) => e.hash,
        }
    }
}

/// First observation of a transaction hash from an ingest source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxSeen {
    pub hash: TxHash,
    pub peer_id: PeerId,
    pub seen_at_unix_ms: i64,
    pub seen_at_mono_ns: u64,
}

/// Notification that a full transaction payload was fetched.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxFetched {
    pub hash: TxHash,
    pub fetched_at_unix_ms: i64,
}

/// Decoded transaction fields normalized for downstream consumers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxDecoded {
    pub hash: TxHash,
    pub tx_type: u8,
    pub sender: Address,
    pub nonce: u64,
    #[serde(default)]
    pub chain_id: Option<u64>,
    #[serde(default)]
    pub to: Option<Address>,
    #[serde(default)]
    pub value_wei: Option<u128>,
    #[serde(default)]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    pub gas_price_wei: Option<u128>,
    #[serde(default)]
    pub max_fee_per_gas_wei: Option<u128>,
    #[serde(default)]
    pub max_priority_fee_per_gas_wei: Option<u128>,
    #[serde(default)]
    pub max_fee_per_blob_gas_wei: Option<u128>,
    #[serde(default)]
    pub calldata_len: Option<u32>,
}

/// Scheduler signal that a transaction is ready for execution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxReady {
    pub hash: TxHash,
    pub sender: Address,
    pub nonce: u64,
}

/// Scheduler signal that a transaction is blocked behind a nonce gap.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxBlocked {
    pub hash: TxHash,
    pub sender: Address,
    pub nonce: u64,
    #[serde(default)]
    pub expected_nonce: Option<u64>,
}

/// Searcher candidate produced from one or more transactions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CandidateQueued {
    pub candidate_id: String,
    pub tx_hash: TxHash,
    #[serde(default)]
    pub member_tx_hashes: Vec<TxHash>,
    #[serde(default)]
    pub chain_id: Option<u64>,
    pub strategy: String,
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
    pub detected_unix_ms: i64,
}

/// Simulation task dispatched for a searcher candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimDispatched {
    pub candidate_id: String,
    pub tx_hash: TxHash,
    #[serde(default)]
    pub member_tx_hashes: Vec<TxHash>,
    pub block_number: u64,
}

/// Opportunity detected for a transaction or bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OppDetected {
    pub hash: TxHash,
    pub strategy: String,
    pub score: u32,
    pub protocol: String,
    pub category: String,
    pub feature_engine_version: String,
    pub scorer_version: String,
    pub strategy_version: String,
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// Completed simulation result attached to a transaction hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimCompleted {
    pub hash: TxHash,
    pub sim_id: String,
    pub status: String,
    pub feature_engine_version: String,
    pub scorer_version: String,
    pub strategy_version: String,
    #[serde(default)]
    pub fail_category: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub tx_count: Option<u32>,
}

/// Builder assembly decision applied to one candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssemblyDecisionApplied {
    pub candidate_id: String,
    pub tx_hash: TxHash,
    pub decision: String,
    #[serde(default)]
    pub replaced_candidate_ids: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
    pub block_number: u64,
}

/// Relay submission result for a bundle or block template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BundleSubmitted {
    pub hash: TxHash,
    pub bundle_id: String,
    pub sim_id: String,
    pub relay: String,
    pub accepted: bool,
    pub feature_engine_version: String,
    pub scorer_version: String,
    pub strategy_version: String,
}

/// Replacement relation between two transaction hashes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxReplaced {
    pub hash: TxHash,
    pub replaced_by: TxHash,
}

/// Drop classification for a transaction hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxDropped {
    pub hash: TxHash,
    pub reason: String,
}

/// Confirmation record for a transaction hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxConfirmed {
    pub hash: TxHash,
    pub block_number: u64,
    pub block_hash: BlockHash,
}

/// Reorg record describing the block transition for a transaction hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxReorged {
    pub hash: TxHash,
    pub old_block_hash: BlockHash,
    pub new_block_hash: BlockHash,
}

/// Compares two events using the canonical deterministic order.
pub fn cmp_deterministic(a: &EventEnvelope, b: &EventEnvelope) -> Ordering {
    a.order_key().cmp(&b.order_key())
}

/// Sorts a slice of events in canonical deterministic order.
pub fn sort_deterministic(events: &mut [EventEnvelope]) {
    events.sort_by(cmp_deterministic);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_str, to_string};

    fn hash(value: u8) -> TxHash {
        [value; 32]
    }

    #[test]
    fn deterministic_ordering_uses_seq_then_source_then_hash() {
        let mut events = vec![
            EventEnvelope {
                seq_id: 10,
                ingest_ts_unix_ms: 1_700_000_001_000,
                ingest_ts_mono_ns: 100,
                source_id: SourceId::new("peer-b"),
                payload: EventPayload::TxFetched(TxFetched {
                    hash: hash(2),
                    fetched_at_unix_ms: 1_700_000_001_000,
                }),
            },
            EventEnvelope {
                seq_id: 9,
                ingest_ts_unix_ms: 1_700_000_000_999,
                ingest_ts_mono_ns: 99,
                source_id: SourceId::new("peer-z"),
                payload: EventPayload::TxFetched(TxFetched {
                    hash: hash(3),
                    fetched_at_unix_ms: 1_700_000_000_999,
                }),
            },
            EventEnvelope {
                seq_id: 10,
                ingest_ts_unix_ms: 1_700_000_001_001,
                ingest_ts_mono_ns: 101,
                source_id: SourceId::new("peer-a"),
                payload: EventPayload::TxFetched(TxFetched {
                    hash: hash(4),
                    fetched_at_unix_ms: 1_700_000_001_001,
                }),
            },
        ];

        sort_deterministic(&mut events);

        assert_eq!(events[0].seq_id, 9);
        assert_eq!(events[1].source_id, SourceId::new("peer-a"));
        assert_eq!(events[2].source_id, SourceId::new("peer-b"));
    }

    #[test]
    fn event_payload_round_trip_json() {
        let event = EventEnvelope {
            seq_id: 42,
            ingest_ts_unix_ms: 1_700_000_123_456,
            ingest_ts_mono_ns: 9_999_999,
            source_id: SourceId::new("rpc-mainnet"),
            payload: EventPayload::TxSeen(TxSeen {
                hash: hash(7),
                peer_id: "peer-123".to_owned(),
                seen_at_unix_ms: 1_700_000_123_456,
                seen_at_mono_ns: 9_999_999,
            }),
        };

        let encoded = to_string(&event).expect("serialize event");
        let decoded: EventEnvelope = from_str(&encoded).expect("deserialize event");

        assert_eq!(event, decoded);
    }

    #[test]
    fn opp_detected_payload_round_trip_json_with_versions() {
        let event = EventEnvelope {
            seq_id: 77,
            ingest_ts_unix_ms: 1_700_000_223_456,
            ingest_ts_mono_ns: 7_777_777,
            source_id: SourceId::new("searcher"),
            payload: EventPayload::OppDetected(OppDetected {
                hash: hash(9),
                strategy: "SandwichCandidate".to_owned(),
                score: 12_345,
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                feature_engine_version: "feature-engine.v1".to_owned(),
                scorer_version: "scorer.v1".to_owned(),
                strategy_version: "strategy.sandwich.v1".to_owned(),
                reasons: vec!["mev_score=90*120".to_owned()],
            }),
        };

        let encoded = to_string(&event).expect("serialize event");
        let decoded: EventEnvelope = from_str(&encoded).expect("deserialize event");

        assert_eq!(event, decoded);
        assert_eq!(decoded.payload.primary_hash(), hash(9));
    }

    #[test]
    fn global_sequencer_assigns_monotonic_ids_and_can_resume() {
        let mut fresh = GlobalSequencer::default();
        assert_eq!(fresh.next_seq_id(), 1);
        assert_eq!(fresh.next_seq_id(), 2);

        let mut resumed = GlobalSequencer::from_latest_seq_id(Some(41));
        assert_eq!(resumed.next_seq_id(), 42);
        assert_eq!(resumed.next_seq_id(), 43);
    }
}
