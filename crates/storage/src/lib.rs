use anyhow::{anyhow, Result};
use common::{Address, PeerId, TxHash};
use event_log::EventEnvelope;
use replay::ReplayFrame;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxSeenRecord {
    pub hash: TxHash,
    pub peer: PeerId,
    pub first_seen_unix_ms: i64,
    pub first_seen_mono_ns: u64,
    pub seen_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxFullRecord {
    pub hash: TxHash,
    pub sender: Address,
    pub nonce: u64,
    pub raw_tx: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxFeaturesRecord {
    pub hash: TxHash,
    pub protocol: String,
    pub category: String,
    pub mev_score: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxLifecycleRecord {
    pub hash: TxHash,
    pub status: String,
    pub reason: Option<String>,
    pub updated_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerStatsRecord {
    pub peer: PeerId,
    pub throughput_tps: u32,
    pub drop_rate_bps: u16,
    pub rtt_ms: u32,
}

pub trait EventStore {
    fn append_event(&mut self, event: EventEnvelope);
    fn list_events(&self) -> &[EventEnvelope];
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryStorage {
    events: Vec<EventEnvelope>,
    tx_seen: Vec<TxSeenRecord>,
    tx_full: Vec<TxFullRecord>,
    tx_features: Vec<TxFeaturesRecord>,
    tx_lifecycle: Vec<TxLifecycleRecord>,
    peer_stats: Vec<PeerStatsRecord>,
    write_latency_ns: Vec<u64>,
}

impl EventStore for InMemoryStorage {
    fn append_event(&mut self, event: EventEnvelope) {
        let start = Instant::now();
        self.events.push(event);
        self.write_latency_ns.push(start.elapsed().as_nanos() as u64);
    }

    fn list_events(&self) -> &[EventEnvelope] {
        &self.events
    }
}

impl InMemoryStorage {
    pub fn upsert_tx_seen(&mut self, record: TxSeenRecord) {
        let start = Instant::now();
        self.tx_seen.push(record);
        self.write_latency_ns.push(start.elapsed().as_nanos() as u64);
    }

    pub fn upsert_tx_full(&mut self, record: TxFullRecord) {
        let start = Instant::now();
        self.tx_full.push(record);
        self.write_latency_ns.push(start.elapsed().as_nanos() as u64);
    }

    pub fn upsert_tx_features(&mut self, record: TxFeaturesRecord) {
        let start = Instant::now();
        self.tx_features.push(record);
        self.write_latency_ns.push(start.elapsed().as_nanos() as u64);
    }

    pub fn upsert_tx_lifecycle(&mut self, record: TxLifecycleRecord) {
        let start = Instant::now();
        self.tx_lifecycle.push(record);
        self.write_latency_ns.push(start.elapsed().as_nanos() as u64);
    }

    pub fn upsert_peer_stats(&mut self, record: PeerStatsRecord) {
        let start = Instant::now();
        self.peer_stats.push(record);
        self.write_latency_ns.push(start.elapsed().as_nanos() as u64);
    }

    pub fn tx_seen(&self) -> &[TxSeenRecord] {
        &self.tx_seen
    }

    pub fn tx_full(&self) -> &[TxFullRecord] {
        &self.tx_full
    }

    pub fn tx_features(&self) -> &[TxFeaturesRecord] {
        &self.tx_features
    }

    pub fn tx_lifecycle(&self) -> &[TxLifecycleRecord] {
        &self.tx_lifecycle
    }

    pub fn peer_stats(&self) -> &[PeerStatsRecord] {
        &self.peer_stats
    }

    pub fn avg_write_latency_ns(&self) -> Option<u64> {
        if self.write_latency_ns.is_empty() {
            return None;
        }
        Some(
            self.write_latency_ns
                .iter()
                .sum::<u64>()
                .saturating_div(self.write_latency_ns.len() as u64),
        )
    }
}

pub fn export_replay_frames_json(frames: &[ReplayFrame]) -> Result<String> {
    serde_json::to_string_pretty(frames).map_err(|err| anyhow!(err))
}

pub fn export_replay_frames_csv(frames: &[ReplayFrame]) -> String {
    let mut out = String::from("seq_hi,timestamp_unix_ms,pending_hashes\n");
    for frame in frames {
        let hashes = frame
            .pending
            .iter()
            .map(format_hash)
            .collect::<Vec<_>>()
            .join(";");
        out.push_str(&format!(
            "{},{},{}\n",
            frame.seq_hi, frame.timestamp_unix_ms, hashes
        ));
    }
    out
}

pub trait ParquetExporter {
    fn export_replay_frames_parquet(
        &self,
        _frames: &[ReplayFrame],
        _path: &str,
    ) -> Result<()> {
        Err(anyhow!(
            "parquet export is not wired in this crate yet; use adapter implementation"
        ))
    }
}

#[derive(Clone, Debug, Default)]
pub struct UnsupportedParquetExporter;

impl ParquetExporter for UnsupportedParquetExporter {}

fn format_hash(hash: &TxHash) -> String {
    let mut out = String::from("0x");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::SourceId;
    use event_log::{EventPayload, TxSeen};

    fn hash(v: u8) -> TxHash {
        [v; 32]
    }

    #[test]
    fn in_memory_storage_inserts_and_queries_records() {
        let mut store = InMemoryStorage::default();

        store.upsert_tx_seen(TxSeenRecord {
            hash: hash(1),
            peer: "peer-a".to_owned(),
            first_seen_unix_ms: 1_700_000_000_000,
            first_seen_mono_ns: 10,
            seen_count: 1,
        });
        store.upsert_tx_full(TxFullRecord {
            hash: hash(1),
            sender: [9; 20],
            nonce: 1,
            raw_tx: vec![1, 2, 3],
        });
        store.upsert_tx_features(TxFeaturesRecord {
            hash: hash(1),
            protocol: "uniswap-v2".to_owned(),
            category: "swap".to_owned(),
            mev_score: 80,
        });
        store.upsert_tx_lifecycle(TxLifecycleRecord {
            hash: hash(1),
            status: "pending".to_owned(),
            reason: None,
            updated_unix_ms: 1_700_000_000_001,
        });
        store.upsert_peer_stats(PeerStatsRecord {
            peer: "peer-a".to_owned(),
            throughput_tps: 100,
            drop_rate_bps: 12,
            rtt_ms: 5,
        });

        assert_eq!(store.tx_seen().len(), 1);
        assert_eq!(store.tx_full().len(), 1);
        assert_eq!(store.tx_features().len(), 1);
        assert_eq!(store.tx_lifecycle().len(), 1);
        assert_eq!(store.peer_stats().len(), 1);
        assert!(store.avg_write_latency_ns().is_some());
    }

    #[test]
    fn event_store_appends_events() {
        let mut store = InMemoryStorage::default();
        store.append_event(EventEnvelope {
            seq_id: 1,
            ingest_ts_unix_ms: 1_700_000_000_000,
            ingest_ts_mono_ns: 10,
            source_id: SourceId::new("test"),
            payload: EventPayload::TxSeen(TxSeen {
                hash: hash(2),
                peer_id: "peer-a".to_owned(),
                seen_at_unix_ms: 1_700_000_000_000,
                seen_at_mono_ns: 10,
            }),
        });
        assert_eq!(store.list_events().len(), 1);
    }

    #[test]
    fn replay_export_outputs_are_deterministic() {
        let frames = vec![
            ReplayFrame {
                seq_hi: 1,
                timestamp_unix_ms: 1_700_000_000_000,
                pending: vec![hash(3)],
            },
            ReplayFrame {
                seq_hi: 2,
                timestamp_unix_ms: 1_700_000_000_100,
                pending: vec![hash(3), hash(4)],
            },
        ];
        let json_a = export_replay_frames_json(&frames).expect("json");
        let json_b = export_replay_frames_json(&frames).expect("json");
        assert_eq!(json_a, json_b);

        let csv = export_replay_frames_csv(&frames);
        assert!(csv.contains("seq_hi,timestamp_unix_ms,pending_hashes"));
        assert!(csv.contains("0x03030303"));
    }

    #[test]
    fn unsupported_parquet_adapter_returns_error() {
        let exporter = UnsupportedParquetExporter;
        let frames = vec![ReplayFrame {
            seq_hi: 1,
            timestamp_unix_ms: 1_700_000_000_000,
            pending: vec![hash(9)],
        }];
        let err = exporter
            .export_replay_frames_parquet(&frames, "/tmp/replay.parquet")
            .expect_err("should return adapter error");
        assert!(err
            .to_string()
            .contains("parquet export is not wired in this crate yet"));
    }
}
