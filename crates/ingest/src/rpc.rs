use ahash::RandomState;
use anyhow::Result;
use common::{SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxDropped, TxFetched, TxSeen};
use hashbrown::{HashMap, HashSet};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

type FastMap<K, V> = HashMap<K, V, RandomState>;
type FastSet<T> = HashSet<T, RandomState>;

#[derive(Clone, Debug)]
pub struct RpcIngestConfig {
    pub max_seen_hashes: usize,
    pub pending_batch_capacity: usize,
}

impl Default for RpcIngestConfig {
    fn default() -> Self {
        Self {
            max_seen_hashes: 250_000,
            pending_batch_capacity: 4_096,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RawTransaction {
    pub hash: TxHash,
    pub tx_type: u8,
    pub raw: Vec<u8>,
}

pub trait PendingTxProvider {
    fn pending_hashes(&mut self) -> Result<Vec<TxHash>>;
    fn fetch_transaction(&mut self, hash: TxHash) -> Result<Option<RawTransaction>>;
}

pub trait IngestClock {
    fn now_unix_ms(&mut self) -> i64;
    fn now_mono_ns(&mut self) -> u64;
}

#[derive(Default)]
pub struct SystemClock {
    monotonic_ns: u64,
}

impl IngestClock for SystemClock {
    fn now_unix_ms(&mut self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch");
        now.as_millis() as i64
    }

    fn now_mono_ns(&mut self) -> u64 {
        // Skeleton monotonic clock for deterministic event creation.
        self.monotonic_ns = self.monotonic_ns.saturating_add(1_000_000);
        self.monotonic_ns
    }
}

pub struct RpcIngestService<P, C> {
    provider: P,
    clock: C,
    source_id: SourceId,
    config: RpcIngestConfig,
    seen_hashes: FastSet<TxHash>,
    seen_order: VecDeque<TxHash>,
    next_seq_id: u64,
}

impl<P, C> RpcIngestService<P, C>
where
    P: PendingTxProvider,
    C: IngestClock,
{
    pub fn new(provider: P, source_id: SourceId, clock: C) -> Self {
        Self::with_config(provider, source_id, clock, RpcIngestConfig::default())
    }

    pub fn with_config(
        provider: P,
        source_id: SourceId,
        clock: C,
        config: RpcIngestConfig,
    ) -> Self {
        Self {
            provider,
            clock,
            source_id,
            config: RpcIngestConfig {
                max_seen_hashes: config.max_seen_hashes.max(1),
                pending_batch_capacity: config.pending_batch_capacity.max(1),
            },
            seen_hashes: FastSet::default(),
            seen_order: VecDeque::new(),
            next_seq_id: 1,
        }
    }

    pub fn process_pending_batch(&mut self) -> Result<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        let hashes = self.provider.pending_hashes()?;
        let depth_current = hashes.len();

        for (idx, hash) in hashes.into_iter().enumerate() {
            if idx >= self.config.pending_batch_capacity {
                events.push(self.new_event(EventPayload::TxDropped(TxDropped {
                    hash,
                    reason: self.drop_reason(
                        "QueueFull",
                        "rpc.pending_batch",
                        depth_current,
                        self.config.pending_batch_capacity,
                    ),
                })));
                continue;
            }

            if !self.remember_hash(hash) {
                events.push(self.new_event(EventPayload::TxDropped(TxDropped {
                    hash,
                    reason: self.drop_reason(
                        "Duplicate",
                        "rpc.pending_batch",
                        depth_current,
                        self.config.pending_batch_capacity,
                    ),
                })));
                continue;
            }

            let seen_at_unix_ms = self.clock.now_unix_ms();
            let seen_at_mono_ns = self.clock.now_mono_ns();
            events.push(self.new_event(EventPayload::TxSeen(TxSeen {
                hash,
                peer_id: "rpc-provider".to_owned(),
                seen_at_unix_ms,
                seen_at_mono_ns,
            })));

            if let Some(tx) = self.provider.fetch_transaction(hash)? {
                let fetched_at_unix_ms = self.clock.now_unix_ms();
                events.push(self.new_event(EventPayload::TxFetched(TxFetched {
                    hash,
                    fetched_at_unix_ms,
                })));
                events.push(self.new_event(EventPayload::TxDecoded(TxDecoded {
                    hash: tx.hash,
                    tx_type: tx.tx_type,
                    sender: [0_u8; 20],
                    nonce: 0,
                    chain_id: None,
                    to: None,
                    value_wei: None,
                    gas_limit: None,
                    gas_price_wei: None,
                    max_fee_per_gas_wei: None,
                    max_priority_fee_per_gas_wei: None,
                    max_fee_per_blob_gas_wei: None,
                    calldata_len: Some(tx.raw.len() as u32),
                })));
            }
        }

        Ok(events)
    }

    fn drop_reason(
        &self,
        reason: &str,
        queue_name: &str,
        depth_current: usize,
        depth_peak: usize,
    ) -> String {
        format!(
            "{reason};lane=rpc;source={};queue={queue_name};depth_current={depth_current};depth_peak={depth_peak}",
            self.source_id.0
        )
    }

    fn remember_hash(&mut self, hash: TxHash) -> bool {
        if !self.seen_hashes.insert(hash) {
            return false;
        }
        self.seen_order.push_back(hash);

        while self.seen_order.len() > self.config.max_seen_hashes {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen_hashes.remove(&oldest);
            }
        }
        true
    }

    fn new_event(&mut self, payload: EventPayload) -> EventEnvelope {
        let seq_id = self.next_seq_id;
        self.next_seq_id += 1;

        EventEnvelope {
            seq_id,
            ingest_ts_unix_ms: self.clock.now_unix_ms(),
            ingest_ts_mono_ns: self.clock.now_mono_ns(),
            source_id: self.source_id.clone(),
            payload,
        }
    }
}

pub struct InMemoryPendingTxProvider {
    batches: VecDeque<Vec<TxHash>>,
    tx_by_hash: FastMap<TxHash, RawTransaction>,
}

impl InMemoryPendingTxProvider {
    pub fn new(batches: Vec<Vec<TxHash>>, transactions: Vec<RawTransaction>) -> Self {
        let tx_by_hash = transactions.into_iter().map(|tx| (tx.hash, tx)).collect();
        Self {
            batches: batches.into_iter().collect(),
            tx_by_hash,
        }
    }
}

impl PendingTxProvider for InMemoryPendingTxProvider {
    fn pending_hashes(&mut self) -> Result<Vec<TxHash>> {
        Ok(self.batches.pop_front().unwrap_or_default())
    }

    fn fetch_transaction(&mut self, hash: TxHash) -> Result<Option<RawTransaction>> {
        Ok(self.tx_by_hash.get(&hash).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct StepClock {
        unix_ms: i64,
        mono_ns: u64,
    }

    impl IngestClock for StepClock {
        fn now_unix_ms(&mut self) -> i64 {
            self.unix_ms += 1;
            self.unix_ms
        }

        fn now_mono_ns(&mut self) -> u64 {
            self.mono_ns += 1;
            self.mono_ns
        }
    }

    fn hash(value: u8) -> TxHash {
        [value; 32]
    }

    fn tx(value: u8, tx_type: u8) -> RawTransaction {
        RawTransaction {
            hash: hash(value),
            tx_type,
            raw: vec![value; 4],
        }
    }

    #[test]
    fn deduplicates_hashes_within_batch_and_emits_ordered_events() {
        let provider = InMemoryPendingTxProvider::new(
            vec![vec![hash(1), hash(1), hash(2)]],
            vec![tx(1, 2), tx(2, 2)],
        );
        let clock = StepClock::default();
        let mut service = RpcIngestService::new(provider, SourceId::new("rpc-mainnet"), clock);

        let events = service.process_pending_batch().expect("process batch");

        // 2 unique hashes * (seen + fetched + decoded) events + 1 duplicate drop event
        assert_eq!(events.len(), 7);
        assert!(events.windows(2).all(|w| w[0].seq_id < w[1].seq_id));

        let seen_hashes: Vec<TxHash> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TxSeen(seen) => Some(seen.hash),
                _ => None,
            })
            .collect();

        assert_eq!(seen_hashes, vec![hash(1), hash(2)]);
        assert!(events.iter().any(|event| {
            matches!(
                event.payload,
                EventPayload::TxDropped(ref dropped)
                    if dropped.hash == hash(1)
                        && dropped.reason.contains("Duplicate")
                        && dropped.reason.contains("queue=rpc.pending_batch")
            )
        }));
    }

    #[test]
    fn ignores_hashes_seen_in_previous_batches() {
        let provider = InMemoryPendingTxProvider::new(
            vec![vec![hash(7)], vec![hash(7), hash(8)]],
            vec![tx(7, 2), tx(8, 2)],
        );
        let clock = StepClock::default();
        let mut service = RpcIngestService::new(provider, SourceId::new("rpc-mainnet"), clock);

        let first_batch = service.process_pending_batch().expect("batch one");
        let second_batch = service.process_pending_batch().expect("batch two");

        assert_eq!(first_batch.len(), 3);
        assert_eq!(second_batch.len(), 4);

        let second_seen: Vec<TxHash> = second_batch
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TxSeen(seen) => Some(seen.hash),
                _ => None,
            })
            .collect();

        assert_eq!(second_seen, vec![hash(8)]);
        assert!(second_batch.iter().any(|event| {
            matches!(
                event.payload,
                EventPayload::TxDropped(ref dropped)
                    if dropped.hash == hash(7)
                        && dropped.reason.contains("Duplicate")
                        && dropped.reason.contains("queue=rpc.pending_batch")
            )
        }));
    }

    #[test]
    fn evicts_old_seen_hashes_when_cache_capacity_is_reached() {
        let provider = InMemoryPendingTxProvider::new(
            vec![vec![hash(1)], vec![hash(2)], vec![hash(3)], vec![hash(1)]],
            vec![tx(1, 2), tx(2, 2), tx(3, 2)],
        );
        let clock = StepClock::default();
        let mut service = RpcIngestService::with_config(
            provider,
            SourceId::new("rpc-mainnet"),
            clock,
            RpcIngestConfig {
                max_seen_hashes: 2,
                pending_batch_capacity: 16,
            },
        );

        service.process_pending_batch().expect("batch one");
        service.process_pending_batch().expect("batch two");
        service.process_pending_batch().expect("batch three");
        let fourth_batch = service.process_pending_batch().expect("batch four");

        let seen_hashes: Vec<TxHash> = fourth_batch
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TxSeen(seen) => Some(seen.hash),
                _ => None,
            })
            .collect();
        assert_eq!(seen_hashes, vec![hash(1)]);
    }

    #[test]
    fn emits_queue_full_drop_when_pending_batch_exceeds_capacity() {
        let provider = InMemoryPendingTxProvider::new(
            vec![vec![hash(1), hash(2), hash(3)]],
            vec![tx(1, 2), tx(2, 2), tx(3, 2)],
        );
        let clock = StepClock::default();
        let mut service = RpcIngestService::with_config(
            provider,
            SourceId::new("rpc-mainnet"),
            clock,
            RpcIngestConfig {
                max_seen_hashes: 16,
                pending_batch_capacity: 2,
            },
        );

        let events = service.process_pending_batch().expect("batch");
        let dropped = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TxDropped(dropped) => Some(dropped),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].hash, hash(3));
        assert!(dropped[0].reason.contains("QueueFull"));
        assert!(dropped[0].reason.contains("queue=rpc.pending_batch"));
    }
}
