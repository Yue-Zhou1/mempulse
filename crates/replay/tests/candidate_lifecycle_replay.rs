use common::SourceId;
use event_log::{
    AssemblyDecisionApplied, CandidateQueued, EventEnvelope, EventPayload, SimCompleted,
    SimDispatched,
};

fn hash(value: u8) -> [u8; 32] {
    [value; 32]
}

fn envelope(seq_id: u64, payload: EventPayload) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: 1_700_000_000_000 + seq_id as i64,
        ingest_ts_mono_ns: seq_id * 1_000,
        source_id: SourceId::new("replay-test"),
        payload,
    }
}

fn candidate_queued_event(candidate_id: &str) -> EventEnvelope {
    envelope(
        1,
        EventPayload::CandidateQueued(CandidateQueued {
            candidate_id: candidate_id.to_owned(),
            tx_hash: hash(0x11),
            member_tx_hashes: vec![hash(0x11)],
            strategy: "SandwichCandidate".to_owned(),
            score: 12_345,
            detected_unix_ms: 1_700_000_000_100,
            chain_id: Some(1),
            protocol: "uniswap-v2".to_owned(),
            category: "swap".to_owned(),
            feature_engine_version: "feature-engine.test".to_owned(),
            scorer_version: "scorer.test".to_owned(),
            strategy_version: "strategy.test".to_owned(),
            reasons: vec!["test".to_owned()],
        }),
    )
}

fn sim_dispatched_event(candidate_id: &str) -> EventEnvelope {
    envelope(
        2,
        EventPayload::SimDispatched(SimDispatched {
            candidate_id: candidate_id.to_owned(),
            tx_hash: hash(0x11),
            member_tx_hashes: vec![hash(0x11)],
            block_number: 42,
        }),
    )
}

fn sim_completed_event(candidate_id: &str, status: &str) -> EventEnvelope {
    let _ = candidate_id;
    envelope(
        3,
        EventPayload::SimCompleted(SimCompleted {
            hash: hash(0x11),
            sim_id: "sim-1".to_owned(),
            status: status.to_owned(),
            feature_engine_version: "feature-engine.test".to_owned(),
            scorer_version: "scorer.test".to_owned(),
            strategy_version: "strategy.test".to_owned(),
            fail_category: None,
            latency_ms: Some(12),
            tx_count: Some(1),
        }),
    )
}

fn assembly_decision_event(candidate_id: &str, decision: &str) -> EventEnvelope {
    envelope(
        4,
        EventPayload::AssemblyDecisionApplied(AssemblyDecisionApplied {
            candidate_id: candidate_id.to_owned(),
            tx_hash: hash(0x11),
            decision: decision.to_owned(),
            replaced_candidate_ids: Vec::new(),
            reason: None,
            block_number: 42,
        }),
    )
}

#[test]
fn replay_reconstructs_candidate_simulation_and_assembly_decisions() {
    let events = vec![
        candidate_queued_event("cand-1"),
        sim_dispatched_event("cand-1"),
        sim_completed_event("cand-1", "ok"),
        assembly_decision_event("cand-1", "inserted"),
    ];

    let snapshot = replay::candidate_lifecycle_snapshot(&events, 4).expect("snapshot");
    assert_eq!(
        snapshot.candidates["cand-1"].simulation_status.as_deref(),
        Some("ok")
    );
    assert_eq!(
        snapshot.candidates["cand-1"].assembly_status.as_deref(),
        Some("inserted")
    );
}
