use common::{Address, SourceId, TxHash};
use event_log::{
    AssemblyDecisionApplied, CandidateQueued, EventEnvelope, EventPayload, SimDispatched, TxDecoded,
};

fn hash(value: u8) -> TxHash {
    [value; 32]
}

fn address(value: u8) -> Address {
    [value; 20]
}

#[test]
fn tx_schema_roundtrip_preserves_extended_fields() {
    let envelope = EventEnvelope {
        seq_id: 42,
        ingest_ts_unix_ms: 1_700_000_123_456,
        ingest_ts_mono_ns: 9_999_999,
        source_id: SourceId::new("schema-test"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash(0xaa),
            tx_type: 2,
            sender: address(0xbb),
            nonce: 17,
            chain_id: Some(1),
            to: Some(address(0xcc)),
            value_wei: Some(1_000_000_000_000_000_000),
            gas_limit: Some(180_000),
            gas_price_wei: Some(45_000_000_000),
            max_fee_per_gas_wei: Some(60_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: Some(3),
            calldata_len: Some(196),
        }),
    };

    let encoded = serde_json::to_vec(&envelope).expect("serialize envelope");
    let decoded: EventEnvelope = serde_json::from_slice(&encoded).expect("deserialize envelope");

    assert_eq!(decoded, envelope);
}

#[test]
fn tx_schema_roundtrip_includes_candidate_and_builder_lifecycle_payloads() {
    let queued = EventPayload::CandidateQueued(CandidateQueued {
        candidate_id: "cand-1".to_owned(),
        tx_hash: hash(0x11),
        member_tx_hashes: vec![hash(0x11), hash(0x12)],
        chain_id: Some(1),
        strategy: "SandwichCandidate".to_owned(),
        score: 12_345,
        protocol: "uniswap-v2".to_owned(),
        category: "swap".to_owned(),
        feature_engine_version: "feature-engine.test".to_owned(),
        scorer_version: "scorer.test".to_owned(),
        strategy_version: "strategy.test".to_owned(),
        reasons: vec!["test".to_owned()],
        detected_unix_ms: 1_700_000_000_100,
    });
    let dispatched = EventPayload::SimDispatched(SimDispatched {
        candidate_id: "cand-1".to_owned(),
        tx_hash: hash(0x11),
        member_tx_hashes: vec![hash(0x11), hash(0x12)],
        block_number: 42,
    });
    let applied = EventPayload::AssemblyDecisionApplied(AssemblyDecisionApplied {
        candidate_id: "cand-1".to_owned(),
        tx_hash: hash(0x11),
        decision: "inserted".to_owned(),
        replaced_candidate_ids: Vec::new(),
        reason: None,
        block_number: 42,
    });

    for payload in [queued, dispatched, applied] {
        let json = serde_json::to_string(&payload).expect("serialize payload");
        let decoded: EventPayload = serde_json::from_str(&json).expect("deserialize payload");
        assert_eq!(decoded, payload);
    }
}
