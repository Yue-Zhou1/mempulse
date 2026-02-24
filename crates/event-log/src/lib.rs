use common::{Address, BlockHash, PeerId, SourceId, TxHash};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub seq_id: u64,
    pub ingest_ts_unix_ms: i64,
    pub ingest_ts_mono_ns: u64,
    pub source_id: SourceId,
    pub payload: EventPayload,
}

impl EventEnvelope {
    pub fn order_key(&self) -> EventOrderKey {
        EventOrderKey {
            seq_id: self.seq_id,
            source_id: self.source_id.clone(),
            hash: self.payload.primary_hash(),
        }
    }
}

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
    pub fn from_latest_seq_id(latest_seq_id: Option<u64>) -> Self {
        let next_seq_id = latest_seq_id.unwrap_or(0).saturating_add(1).max(1);
        Self { next_seq_id }
    }

    pub fn next_seq_id(&mut self) -> u64 {
        let seq_id = self.next_seq_id;
        self.next_seq_id = self.next_seq_id.saturating_add(1);
        seq_id
    }

    pub fn assign(&mut self, mut event: EventEnvelope) -> EventEnvelope {
        event.seq_id = self.next_seq_id();
        event
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EventOrderKey {
    pub seq_id: u64,
    pub source_id: SourceId,
    pub hash: TxHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    TxSeen(TxSeen),
    TxFetched(TxFetched),
    TxDecoded(TxDecoded),
    TxReplaced(TxReplaced),
    TxDropped(TxDropped),
    TxConfirmedProvisional(TxConfirmed),
    TxConfirmedFinal(TxConfirmed),
    TxReorged(TxReorged),
}

impl EventPayload {
    pub fn primary_hash(&self) -> TxHash {
        match self {
            EventPayload::TxSeen(e) => e.hash,
            EventPayload::TxFetched(e) => e.hash,
            EventPayload::TxDecoded(e) => e.hash,
            EventPayload::TxReplaced(e) => e.hash,
            EventPayload::TxDropped(e) => e.hash,
            EventPayload::TxConfirmedProvisional(e) => e.hash,
            EventPayload::TxConfirmedFinal(e) => e.hash,
            EventPayload::TxReorged(e) => e.hash,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxSeen {
    pub hash: TxHash,
    pub peer_id: PeerId,
    pub seen_at_unix_ms: i64,
    pub seen_at_mono_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxFetched {
    pub hash: TxHash,
    pub fetched_at_unix_ms: i64,
}

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxReplaced {
    pub hash: TxHash,
    pub replaced_by: TxHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxDropped {
    pub hash: TxHash,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxConfirmed {
    pub hash: TxHash,
    pub block_number: u64,
    pub block_hash: BlockHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxReorged {
    pub hash: TxHash,
    pub old_block_hash: BlockHash,
    pub new_block_hash: BlockHash,
}

pub fn cmp_deterministic(a: &EventEnvelope, b: &EventEnvelope) -> Ordering {
    a.order_key().cmp(&b.order_key())
}

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
    fn global_sequencer_assigns_monotonic_ids_and_can_resume() {
        let mut fresh = GlobalSequencer::default();
        assert_eq!(fresh.next_seq_id(), 1);
        assert_eq!(fresh.next_seq_id(), 2);

        let mut resumed = GlobalSequencer::from_latest_seq_id(Some(41));
        assert_eq!(resumed.next_seq_id(), 42);
        assert_eq!(resumed.next_seq_id(), 43);
    }
}
