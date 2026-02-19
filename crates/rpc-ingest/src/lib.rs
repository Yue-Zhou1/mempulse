use anyhow::Result;
use common::{SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxFetched, TxSeen};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

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

pub struct SystemClock {
    monotonic_ns: u64,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self { monotonic_ns: 0 }
    }
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
    seen_hashes: HashSet<TxHash>,
    next_seq_id: u64,
}

impl<P, C> RpcIngestService<P, C>
where
    P: PendingTxProvider,
    C: IngestClock,
{
    pub fn new(provider: P, source_id: SourceId, clock: C) -> Self {
        Self {
            provider,
            clock,
            source_id,
            seen_hashes: HashSet::new(),
            next_seq_id: 1,
        }
    }

    pub fn process_pending_batch(&mut self) -> Result<Vec<EventEnvelope>> {
        let mut events = Vec::new();
        let hashes = self.provider.pending_hashes()?;

        for hash in hashes {
            if !self.seen_hashes.insert(hash) {
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
                })));
            }
        }

        Ok(events)
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
    tx_by_hash: HashMap<TxHash, RawTransaction>,
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

        // 2 unique hashes * (seen + fetched + decoded) events
        assert_eq!(events.len(), 6);
        assert!(events.windows(2).all(|w| w[0].seq_id < w[1].seq_id));

        let seen_hashes: Vec<TxHash> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TxSeen(seen) => Some(seen.hash),
                _ => None,
            })
            .collect();

        assert_eq!(seen_hashes, vec![hash(1), hash(2)]);
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
        assert_eq!(second_batch.len(), 3);

        let second_seen: Vec<TxHash> = second_batch
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TxSeen(seen) => Some(seen.hash),
                _ => None,
            })
            .collect();

        assert_eq!(second_seen, vec![hash(8)]);
    }
}
