//! Async runtime facade for the devp2p ingest service.

use common::{PeerId, SourceId, TxHash};
use event_log::EventEnvelope;

use crate::{
    GetPooledTransactionsRequest, P2pIngestConfig, P2pIngestService, P2pMetrics, P2pTxPayload,
};

/// Async interface used by higher-level runtimes to feed devp2p announcements
/// and pooled transactions into the ingest state machine.
#[allow(async_fn_in_trait)]
pub trait P2pRuntime {
    /// Records a batch of announced hashes from one peer and returns the
    /// resulting event-log envelopes.
    async fn ingest_announcements(
        &mut self,
        peer_id: PeerId,
        hashes: Vec<TxHash>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope>;

    /// Records full pooled transactions received from one peer and returns the
    /// corresponding fetched and decoded events.
    async fn ingest_pooled_transactions(
        &mut self,
        peer_id: PeerId,
        txs: Vec<P2pTxPayload>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope>;

    /// Pops the next queued pooled-transaction fetch request, if any.
    fn dequeue_get_pooled_transactions(&mut self) -> Option<GetPooledTransactionsRequest>;

    /// Returns the current devp2p ingest counters.
    fn metrics(&self) -> &P2pMetrics;
}

/// Thin async wrapper around [`P2pIngestService`] for runtime integrations that
/// want trait-based dispatch.
#[derive(Clone, Debug)]
pub struct Devp2pRuntime {
    service: P2pIngestService,
}

impl Devp2pRuntime {
    /// Creates a devp2p runtime with the provided queue and dedup settings.
    pub fn new(config: P2pIngestConfig, source_id: SourceId) -> Self {
        Self {
            service: P2pIngestService::new(config, source_id),
        }
    }

    /// Records a batch of announced hashes from one peer.
    pub async fn ingest_announcements(
        &mut self,
        peer_id: PeerId,
        hashes: Vec<TxHash>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope> {
        self.service
            .handle_new_pooled_transaction_hashes(peer_id, hashes, now_unix_ms, now_mono_ns)
    }

    /// Records full pooled transactions received from one peer.
    pub async fn ingest_pooled_transactions(
        &mut self,
        peer_id: PeerId,
        txs: Vec<P2pTxPayload>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope> {
        self.service
            .handle_pooled_transactions(peer_id, txs, now_unix_ms, now_mono_ns)
    }

    /// Pops the next queued pooled-transaction fetch request, if any.
    pub fn dequeue_get_pooled_transactions(&mut self) -> Option<GetPooledTransactionsRequest> {
        self.service.dequeue_get_pooled_transactions()
    }

    /// Returns the current devp2p ingest counters.
    pub fn metrics(&self) -> &P2pMetrics {
        self.service.metrics()
    }
}

impl P2pRuntime for Devp2pRuntime {
    async fn ingest_announcements(
        &mut self,
        peer_id: PeerId,
        hashes: Vec<TxHash>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope> {
        self.service
            .handle_new_pooled_transaction_hashes(peer_id, hashes, now_unix_ms, now_mono_ns)
    }

    async fn ingest_pooled_transactions(
        &mut self,
        peer_id: PeerId,
        txs: Vec<P2pTxPayload>,
        now_unix_ms: i64,
        now_mono_ns: u64,
    ) -> Vec<EventEnvelope> {
        self.service
            .handle_pooled_transactions(peer_id, txs, now_unix_ms, now_mono_ns)
    }

    fn dequeue_get_pooled_transactions(&mut self) -> Option<GetPooledTransactionsRequest> {
        self.service.dequeue_get_pooled_transactions()
    }

    fn metrics(&self) -> &P2pMetrics {
        self.service.metrics()
    }
}
