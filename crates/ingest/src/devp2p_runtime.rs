use common::{PeerId, SourceId, TxHash};
use event_log::EventEnvelope;

use crate::{
    GetPooledTransactionsRequest, P2pIngestConfig, P2pIngestService, P2pMetrics, P2pTxPayload,
};

#[derive(Clone, Debug)]
pub struct Devp2pRuntime {
    service: P2pIngestService,
}

impl Devp2pRuntime {
    pub fn new(config: P2pIngestConfig, source_id: SourceId) -> Self {
        Self {
            service: P2pIngestService::new(config, source_id),
        }
    }

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

    pub fn dequeue_get_pooled_transactions(&mut self) -> Option<GetPooledTransactionsRequest> {
        self.service.dequeue_get_pooled_transactions()
    }

    pub fn metrics(&self) -> &P2pMetrics {
        self.service.metrics()
    }
}
