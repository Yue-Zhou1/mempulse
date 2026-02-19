use common::{PeerId, SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxFetched, TxSeen};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug)]
pub struct P2pIngestConfig {
    pub fetch_queue_capacity: usize,
}

impl Default for P2pIngestConfig {
    fn default() -> Self {
        Self {
            fetch_queue_capacity: 4_096,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct P2pMetrics {
    pub announcements_total: u64,
    pub duplicates_dropped_total: u64,
    pub queue_dropped_total: u64,
    pub tx_full_received_total: u64,
    pub tx_decode_emitted_total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropagationStats {
    pub count: u64,
    pub avg_delay_ms: u32,
    pub p99_delay_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetPooledTransactionsRequest {
    pub peer_id: PeerId,
    pub hashes: Vec<TxHash>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct P2pTxPayload {
    pub hash: TxHash,
    pub tx_type: u8,
    pub sender: [u8; 20],
    pub nonce: u64,
}

#[derive(Clone, Debug)]
struct FirstSeen {
    unix_ms: i64,
}

#[derive(Clone, Debug)]
pub struct P2pIngestService {
    config: P2pIngestConfig,
    source_id: SourceId,
    seq_id: u64,
    seen_hashes: HashSet<TxHash>,
    first_seen: HashMap<TxHash, FirstSeen>,
    fetch_queue: VecDeque<GetPooledTransactionsRequest>,
    propagation_delays_by_peer: HashMap<PeerId, Vec<u32>>,
    metrics: P2pMetrics,
}

impl P2pIngestService {
    pub fn new(config: P2pIngestConfig, source_id: SourceId) -> Self {
        Self {
            config,
            source_id,
            seq_id: 1,
            seen_hashes: HashSet::new(),
            first_seen: HashMap::new(),
            fetch_queue: VecDeque::new(),
            propagation_delays_by_peer: HashMap::new(),
            metrics: P2pMetrics::default(),
        }
    }

    pub fn handle_new_pooled_transaction_hashes(
        &mut self,
        peer_id: PeerId,
        hashes: Vec<TxHash>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope> {
        let mut events = Vec::new();
        self.metrics.announcements_total += hashes.len() as u64;

        for hash in hashes {
            if let Some(first_seen) = self.first_seen.get(&hash) {
                self.metrics.duplicates_dropped_total += 1;
                let delay = now_unix_ms.saturating_sub(first_seen.unix_ms) as u32;
                self.propagation_delays_by_peer
                    .entry(peer_id.clone())
                    .or_default()
                    .push(delay);
                continue;
            }

            if self.fetch_queue.len() >= self.config.fetch_queue_capacity {
                self.metrics.queue_dropped_total += 1;
                continue;
            }

            self.seen_hashes.insert(hash);
            self.first_seen.insert(
                hash,
                FirstSeen {
                    unix_ms: now_unix_ms,
                },
            );
            self.fetch_queue.push_back(GetPooledTransactionsRequest {
                peer_id: peer_id.clone(),
                hashes: vec![hash],
            });
            events.push(self.new_event(
                now_unix_ms,
                now_mono_ns,
                EventPayload::TxSeen(TxSeen {
                    hash,
                    peer_id: peer_id.clone(),
                    seen_at_unix_ms: now_unix_ms,
                    seen_at_mono_ns: now_mono_ns,
                }),
            ));
        }

        events
    }

    pub fn dequeue_get_pooled_transactions(&mut self) -> Option<GetPooledTransactionsRequest> {
        self.fetch_queue.pop_front()
    }

    pub fn handle_pooled_transactions(
        &mut self,
        _peer_id: PeerId,
        txs: Vec<P2pTxPayload>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope> {
        let mut events = Vec::with_capacity(txs.len() * 2);
        for tx in txs {
            self.metrics.tx_full_received_total += 1;
            events.push(self.new_event(
                now_unix_ms,
                now_mono_ns,
                EventPayload::TxFetched(TxFetched {
                    hash: tx.hash,
                    fetched_at_unix_ms: now_unix_ms,
                }),
            ));
            self.metrics.tx_decode_emitted_total += 1;
            events.push(self.new_event(
                now_unix_ms,
                now_mono_ns,
                EventPayload::TxDecoded(TxDecoded {
                    hash: tx.hash,
                    tx_type: tx.tx_type,
                    sender: tx.sender,
                    nonce: tx.nonce,
                }),
            ));
        }
        events
    }

    pub fn propagation_stats_by_peer(&self) -> HashMap<PeerId, PropagationStats> {
        self.propagation_delays_by_peer
            .iter()
            .map(|(peer, delays)| {
                let mut sorted = delays.clone();
                sorted.sort_unstable();
                let count = sorted.len() as u64;
                let avg_delay_ms = if sorted.is_empty() {
                    0
                } else {
                    sorted.iter().map(|value| *value as u64).sum::<u64>() as u32 / sorted.len() as u32
                };
                let p99_delay_ms = if sorted.is_empty() {
                    0
                } else {
                    let idx = ((sorted.len() - 1) as f64 * 0.99).round() as usize;
                    sorted[idx]
                };
                (
                    peer.clone(),
                    PropagationStats {
                        count,
                        avg_delay_ms,
                        p99_delay_ms,
                    },
                )
            })
            .collect()
    }

    pub fn metrics(&self) -> &P2pMetrics {
        &self.metrics
    }

    fn new_event(
        &mut self,
        now_unix_ms: i64,
        now_mono_ns: u64,
        payload: EventPayload,
    ) -> EventEnvelope {
        let seq_id = self.seq_id;
        self.seq_id += 1;
        EventEnvelope {
            seq_id,
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: now_mono_ns,
            source_id: self.source_id.clone(),
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u8) -> TxHash {
        [value; 32]
    }

    #[test]
    fn dedup_tracks_first_seen_and_propagation_delay() {
        let mut service = P2pIngestService::new(
            P2pIngestConfig {
                fetch_queue_capacity: 8,
            },
            SourceId::new("p2p"),
        );

        let first = service.handle_new_pooled_transaction_hashes(
            "peer-a".to_owned(),
            vec![hash(1)],
            1_700_000_000_000,
            10,
        );
        assert_eq!(first.len(), 1);

        let second = service.handle_new_pooled_transaction_hashes(
            "peer-b".to_owned(),
            vec![hash(1)],
            1_700_000_000_011,
            20,
        );
        assert!(second.is_empty());
        assert_eq!(service.metrics().duplicates_dropped_total, 1);

        let stats = service.propagation_stats_by_peer();
        assert_eq!(stats.get("peer-b").unwrap().count, 1);
        assert_eq!(stats.get("peer-b").unwrap().avg_delay_ms, 11);
    }

    #[test]
    fn backpressure_drops_new_hashes_when_queue_is_full() {
        let mut service = P2pIngestService::new(
            P2pIngestConfig {
                fetch_queue_capacity: 1,
            },
            SourceId::new("p2p"),
        );

        let first = service.handle_new_pooled_transaction_hashes(
            "peer-a".to_owned(),
            vec![hash(1)],
            1_700_000_000_000,
            10,
        );
        let second = service.handle_new_pooled_transaction_hashes(
            "peer-a".to_owned(),
            vec![hash(2)],
            1_700_000_000_001,
            11,
        );

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
        assert_eq!(service.metrics().queue_dropped_total, 1);
    }

    #[test]
    fn pooled_transaction_handler_emits_fetched_and_decoded_events() {
        let mut service = P2pIngestService::new(P2pIngestConfig::default(), SourceId::new("p2p"));
        let events = service.handle_pooled_transactions(
            "peer-a".to_owned(),
            vec![P2pTxPayload {
                hash: hash(9),
                tx_type: 2,
                sender: [7; 20],
                nonce: 5,
            }],
            1_700_000_000_123,
            99,
        );

        assert_eq!(events.len(), 2);
        assert_eq!(service.metrics().tx_full_received_total, 1);
        assert_eq!(service.metrics().tx_decode_emitted_total, 1);
        assert!(matches!(events[0].payload, EventPayload::TxFetched(_)));
        assert!(matches!(events[1].payload, EventPayload::TxDecoded(_)));
    }
}
