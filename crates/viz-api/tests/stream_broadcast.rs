use viz_api::{
    MarketStats, StreamV2Dispatch, StreamV2Patch, StreamV2Watermark,
    stream_broadcast::{DashboardStreamBroadcastEvent, DashboardStreamBroadcaster},
};

fn make_dispatch(seq: u64) -> StreamV2Dispatch {
    StreamV2Dispatch {
        op: "DISPATCH".to_owned(),
        event_type: "DELTA_BATCH".to_owned(),
        seq,
        channel: "tx.main".to_owned(),
        has_gap: false,
        patch: StreamV2Patch {
            upsert: Vec::new(),
            remove: Vec::new(),
            feature_upsert: Vec::new(),
            opportunity_upsert: Vec::new(),
        },
        watermark: StreamV2Watermark {
            latest_ingest_seq: seq,
        },
        market_stats: MarketStats {
            total_signal_volume: 100,
            total_tx_count: 100,
            low_risk_count: 90,
            medium_risk_count: 8,
            high_risk_count: 2,
            success_rate_bps: 9_800,
        },
    }
}

#[tokio::test]
async fn stream_broadcast_fan_out_delivers_ordered_seq_to_multiple_subscribers() {
    let broadcaster = DashboardStreamBroadcaster::new(32, 64);
    let (initial_a, mut rx_a) = broadcaster.subscribe_from(0);
    let (initial_b, mut rx_b) = broadcaster.subscribe_from(0);
    assert!(initial_a.is_empty());
    assert!(initial_b.is_empty());

    for seq in [1_u64, 2, 3] {
        broadcaster.publish_delta(make_dispatch(seq));
    }

    let mut received_a = Vec::new();
    let mut received_b = Vec::new();
    for _ in 0..3 {
        if let DashboardStreamBroadcastEvent::Delta(dispatch) = rx_a.recv().await.expect("rx_a") {
            received_a.push(dispatch.seq);
        }
        if let DashboardStreamBroadcastEvent::Delta(dispatch) = rx_b.recv().await.expect("rx_b") {
            received_b.push(dispatch.seq);
        }
    }

    assert_eq!(received_a, vec![1, 2, 3]);
    assert_eq!(received_b, vec![1, 2, 3]);
}

#[tokio::test]
async fn stream_broadcast_late_subscriber_replays_or_resets_based_on_cursor() {
    let broadcaster = DashboardStreamBroadcaster::new(2, 64);
    for seq in [10_u64, 11, 12] {
        broadcaster.publish_delta(make_dispatch(seq));
    }

    let (resume_events, _rx_resume) = broadcaster.subscribe_from(10);
    let resumed_seqs = resume_events
        .into_iter()
        .filter_map(|event| match event {
            DashboardStreamBroadcastEvent::Delta(dispatch) => Some(dispatch.seq),
            DashboardStreamBroadcastEvent::Reset { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(resumed_seqs, vec![11, 12]);

    let (gap_events, _rx_gap) = broadcaster.subscribe_from(1);
    assert_eq!(gap_events.len(), 1);
    match &gap_events[0] {
        DashboardStreamBroadcastEvent::Reset {
            reason,
            latest_seq_id,
        } => {
            assert_eq!(reason, "gap");
            assert_eq!(*latest_seq_id, 12);
        }
        other => panic!("expected reset event, got {other:?}"),
    }
}
