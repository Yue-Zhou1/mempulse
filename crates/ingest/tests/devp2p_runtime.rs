use common::{SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload};
use ingest::{Devp2pRuntime, P2pIngestConfig, P2pRuntime, P2pTxPayload};

fn hash(value: u8) -> TxHash {
    [value; 32]
}

async fn ingest_seen_via_runtime<R: P2pRuntime>(
    runtime: &mut R,
    peer_id: &str,
    hashes: Vec<TxHash>,
) -> Vec<EventEnvelope> {
    runtime
        .ingest_announcements(peer_id.to_owned(), hashes, 1_700_000_000_000, 100)
        .await
}

#[tokio::test]
async fn devp2p_runtime_emits_seen_fetch_decode_and_tracks_backpressure() {
    let mut runtime = Devp2pRuntime::new(
        P2pIngestConfig {
            fetch_queue_capacity: 1,
            max_seen_hashes: 64,
        },
        SourceId::new("p2p-runtime"),
    );

    let seen_events = ingest_seen_via_runtime(&mut runtime, "peer-a", vec![hash(1), hash(2)]).await;

    assert_eq!(seen_events.len(), 2);
    assert!(matches!(seen_events[0].payload, EventPayload::TxSeen(_)));
    assert!(matches!(
        seen_events[1].payload,
        EventPayload::TxDropped(ref dropped)
            if dropped.hash == hash(2)
                && dropped.reason.contains("QueueFull")
                && dropped.reason.contains("queue=p2p.fetch")
    ));
    assert_eq!(runtime.metrics().queue_dropped_total, 1);

    let req = runtime
        .dequeue_get_pooled_transactions()
        .expect("fetch request");
    assert_eq!(req.peer_id, "peer-a");
    assert_eq!(req.hashes, vec![hash(1)]);

    let tx_events = runtime
        .ingest_pooled_transactions(
            "peer-a".to_owned(),
            vec![P2pTxPayload {
                hash: hash(1),
                tx_type: 2,
                sender: [7_u8; 20],
                nonce: 42,
            }],
            1_700_000_000_010,
            110,
        )
        .await;

    assert_eq!(tx_events.len(), 2);
    assert!(matches!(tx_events[0].payload, EventPayload::TxFetched(_)));
    assert!(matches!(tx_events[1].payload, EventPayload::TxDecoded(_)));
}
