use common::SourceId;
use event_log::{EventEnvelope, EventPayload, TxSeen};
use std::sync::{Arc, RwLock};
use storage::{EventStore, InMemoryStorage, OpportunityRecord, TxFeaturesRecord};
use viz_api::{InMemoryVizProvider, VizDataProvider};

fn hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn seen_event(seq_id: u64, hash_seed: u8) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: 1_700_000_000_000 + seq_id as i64,
        ingest_ts_mono_ns: seq_id * 1_000_000,
        source_id: SourceId::new("rpc-test"),
        payload: EventPayload::TxSeen(TxSeen {
            hash: hash(hash_seed),
            peer_id: "peer-a".to_owned(),
            seen_at_unix_ms: 1_700_000_000_000 + seq_id as i64,
            seen_at_mono_ns: seq_id * 1_000_000,
        }),
    }
}

fn seed_storage() -> Arc<RwLock<InMemoryStorage>> {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let mut guard = storage.write().expect("lock storage for seed");
    for seq in 1..=6 {
        guard.append_event(seen_event(seq, seq as u8));
    }

    guard.upsert_tx_features(TxFeaturesRecord {
        hash: hash(1),
        chain_id: Some(1),
        protocol: "uniswap-v2".to_owned(),
        category: "swap".to_owned(),
        mev_score: 82,
        urgency_score: 73,
        method_selector: Some([0x38, 0xed, 0x17, 0x39]),
        feature_engine_version: "feature-engine.v1".to_owned(),
    });
    guard.upsert_tx_features(TxFeaturesRecord {
        hash: hash(2),
        chain_id: Some(8453),
        protocol: "aerodrome".to_owned(),
        category: "swap".to_owned(),
        mev_score: 76,
        urgency_score: 61,
        method_selector: Some([0x12, 0x34, 0x56, 0x78]),
        feature_engine_version: "feature-engine.v1".to_owned(),
    });

    guard.upsert_opportunity(OpportunityRecord {
        tx_hash: hash(1),
        strategy: "Sandwich".to_owned(),
        score: 91,
        protocol: "uniswap-v2".to_owned(),
        category: "swap".to_owned(),
        chain_id: Some(1),
        feature_engine_version: "feature-engine.v1".to_owned(),
        scorer_version: "scorer.v1".to_owned(),
        strategy_version: "strategy.v1".to_owned(),
        reasons: vec!["high-impact".to_owned()],
        detected_unix_ms: 1_700_000_000_999,
    });
    drop(guard);
    storage
}

#[test]
fn dashboard_snapshot_perf_reuses_cached_aggregates_between_calls() {
    let storage = seed_storage();
    let provider = InMemoryVizProvider::new(storage, Arc::new(Vec::new()), 1);

    let snapshot_a = (
        provider.replay_points(),
        provider.feature_summary(),
        provider.feature_details(50),
        provider.opportunities(50, 0),
    );
    assert_eq!(provider.dashboard_cache_refreshes(), 1);

    let snapshot_b = (
        provider.replay_points(),
        provider.feature_summary(),
        provider.feature_details(50),
        provider.opportunities(50, 0),
    );
    assert_eq!(provider.dashboard_cache_refreshes(), 1);
    assert_eq!(snapshot_a, snapshot_b);
}

#[test]
fn dashboard_snapshot_perf_invalidates_cache_when_storage_advances() {
    let storage = seed_storage();
    let provider = InMemoryVizProvider::new(storage.clone(), Arc::new(Vec::new()), 1);

    let _ = provider.replay_points();
    assert_eq!(provider.dashboard_cache_refreshes(), 1);

    storage
        .write()
        .expect("lock storage for update")
        .append_event(seen_event(7, 7));

    let replay = provider.replay_points();
    assert_eq!(provider.dashboard_cache_refreshes(), 2);
    assert_eq!(replay.last().map(|point| point.seq_hi), Some(7));
}
