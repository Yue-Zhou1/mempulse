use builder::{
    AssemblyCandidate, AssemblyCandidateKind, AssemblyConfig, AssemblyDecision, AssemblyEngine,
    AssemblyRollbackDecision, RelayBuildContext, SimulationApproval,
};

#[test]
fn assembly_tracks_candidate_block_state_and_objective() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let decision = engine.insert(candidate(
        "cand-a",
        vec![hash(0x11)],
        120,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Inserted {
            candidate_id: "cand-a".to_owned(),
            replaced_candidate_ids: Vec::new(),
        }
    );

    let snapshot = engine.snapshot();
    assert_eq!(snapshot.candidates.len(), 1);
    assert_eq!(snapshot.objective.total_priority_score, 120);
    assert_eq!(snapshot.objective.total_gas_used, 21_000);
    assert_eq!(snapshot.objective.total_candidates, 1);
    assert_eq!(snapshot.candidates[0].candidate_id, "cand-a");
    assert_eq!(snapshot.candidates[0].tx_hashes, vec![hash(0x11)]);
}

#[test]
fn assembly_rejects_candidates_that_are_not_simulation_approved() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let decision = engine.insert(candidate(
        "cand-rejected",
        vec![hash(0x22)],
        250,
        42_000,
        AssemblyCandidateKind::Transaction,
        false,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Rejected {
            candidate_id: "cand-rejected".to_owned(),
            reason: "simulation_not_approved".to_owned(),
        }
    );
    assert!(engine.snapshot().candidates.is_empty());
}

#[test]
fn assembly_replaces_conflicting_candidates_when_objective_improves() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    assert_eq!(
        engine.insert(candidate(
            "cand-a",
            vec![hash(0x31)],
            120,
            21_000,
            AssemblyCandidateKind::Transaction,
            true,
        )),
        AssemblyDecision::Inserted {
            candidate_id: "cand-a".to_owned(),
            replaced_candidate_ids: Vec::new(),
        }
    );

    let decision = engine.insert(candidate(
        "cand-bundle",
        vec![hash(0x31), hash(0x32)],
        260,
        45_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Inserted {
            candidate_id: "cand-bundle".to_owned(),
            replaced_candidate_ids: vec!["cand-a".to_owned()],
        }
    );

    let snapshot = engine.snapshot();
    assert_eq!(snapshot.candidates.len(), 1);
    assert_eq!(snapshot.candidates[0].candidate_id, "cand-bundle");
    assert_eq!(snapshot.objective.total_priority_score, 260);
    assert_eq!(snapshot.objective.total_gas_used, 45_000);
}

#[test]
fn assembly_rejects_conflicting_candidate_when_objective_does_not_improve() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    assert_eq!(
        engine.insert(candidate(
            "cand-a",
            vec![hash(0x41)],
            220,
            21_000,
            AssemblyCandidateKind::Transaction,
            true,
        )),
        AssemblyDecision::Inserted {
            candidate_id: "cand-a".to_owned(),
            replaced_candidate_ids: Vec::new(),
        }
    );

    let decision = engine.insert(candidate(
        "cand-weaker-bundle",
        vec![hash(0x41), hash(0x42)],
        180,
        45_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Rejected {
            candidate_id: "cand-weaker-bundle".to_owned(),
            reason: "objective_not_improved".to_owned(),
        }
    );

    let snapshot = engine.snapshot();
    assert_eq!(snapshot.candidates.len(), 1);
    assert_eq!(snapshot.candidates[0].candidate_id, "cand-a");
}

#[test]
fn assembly_rejects_candidate_when_block_gas_limit_would_be_exceeded() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 50_000,
    });

    assert_eq!(
        engine.insert(candidate(
            "cand-a",
            vec![hash(0x51)],
            100,
            30_000,
            AssemblyCandidateKind::Transaction,
            true,
        )),
        AssemblyDecision::Inserted {
            candidate_id: "cand-a".to_owned(),
            replaced_candidate_ids: Vec::new(),
        }
    );

    let decision = engine.insert(candidate(
        "cand-b",
        vec![hash(0x52)],
        150,
        25_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Rejected {
            candidate_id: "cand-b".to_owned(),
            reason: "block_gas_limit_exceeded".to_owned(),
        }
    );
}

#[test]
fn assembly_metrics_capture_insert_replace_and_reject_outcomes() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0x61)],
        120,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));
    let _ = engine.insert(candidate(
        "cand-bundle",
        vec![hash(0x61), hash(0x62)],
        260,
        45_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));
    let _ = engine.insert(candidate(
        "cand-rejected",
        vec![hash(0x63)],
        300,
        21_000,
        AssemblyCandidateKind::Transaction,
        false,
    ));

    let metrics = engine.metrics();
    assert_eq!(metrics.inserted_total, 2);
    assert_eq!(metrics.replaced_total, 1);
    assert_eq!(metrics.rejected_total, 1);
    assert_eq!(metrics.rejected_simulation_not_approved_total, 1);
    assert_eq!(metrics.rejected_objective_not_improved_total, 0);
    assert_eq!(metrics.rejected_gas_limit_total, 0);
    assert_eq!(metrics.active_candidate_total, 1);
    assert_eq!(metrics.total_priority_score, 260);
    assert_eq!(metrics.total_gas_used, 45_000);
    assert!(metrics.last_decision_latency_ns > 0);
    assert!(metrics.max_decision_latency_ns > 0);
    assert!(metrics.total_decision_latency_ns >= metrics.last_decision_latency_ns);
}

#[test]
fn assembly_builds_relay_template_from_current_block_state() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0x71)],
        120,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));
    let _ = engine.insert(candidate(
        "cand-bundle",
        vec![hash(0x72), hash(0x73)],
        260,
        45_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));

    let context = RelayBuildContext {
        slot: 9_001,
        parent_hash: "0xabc".to_owned(),
        builder_pubkey: "0xdef".to_owned(),
    };
    let template = engine
        .build_relay_template(&context)
        .expect("relay template should be available");

    assert_eq!(template.slot, 9_001);
    assert_eq!(template.parent_hash, "0xabc");
    assert_eq!(template.builder_pubkey, "0xdef");
    assert_eq!(template.tx_count, 3);
    assert_eq!(template.gas_used, 66_000);
    assert!(template.block_hash.starts_with("0x"));
    assert_eq!(template.block_hash.len(), 66);

    let second = engine
        .build_relay_template(&context)
        .expect("relay template should remain deterministic");
    assert_eq!(second.block_hash, template.block_hash);
}

#[test]
fn assembly_can_roll_back_candidate_by_id() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0x81)],
        120,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));

    let decision = engine.remove_candidate("cand-a");
    assert_eq!(
        decision,
        AssemblyRollbackDecision::RolledBack {
            candidate_id: "cand-a".to_owned(),
        }
    );

    let snapshot = engine.snapshot();
    assert!(snapshot.candidates.is_empty());
    assert_eq!(snapshot.objective.total_priority_score, 0);
    assert_eq!(snapshot.objective.total_gas_used, 0);
    assert_eq!(engine.metrics().rollback_total, 1);
}

#[test]
fn assembly_replaces_multiple_conflicts_when_objective_improves() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0x91)],
        100,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));
    let _ = engine.insert(candidate(
        "cand-b",
        vec![hash(0x92)],
        120,
        22_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));

    let decision = engine.insert(candidate(
        "cand-bundle",
        vec![hash(0x91), hash(0x92)],
        260,
        50_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Inserted {
            candidate_id: "cand-bundle".to_owned(),
            replaced_candidate_ids: vec!["cand-a".to_owned(), "cand-b".to_owned()],
        }
    );
    assert_eq!(engine.snapshot().candidates.len(), 1);
    assert_eq!(engine.metrics().replaced_total, 2);
}

#[test]
fn assembly_rejects_tied_conflict_with_first_in_wins_policy() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0xa1)],
        200,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));

    let decision = engine.insert(candidate(
        "cand-bundle",
        vec![hash(0xa1), hash(0xa2)],
        200,
        42_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Rejected {
            candidate_id: "cand-bundle".to_owned(),
            reason: "objective_not_improved".to_owned(),
        }
    );
    assert_eq!(engine.snapshot().candidates[0].candidate_id, "cand-a");
    assert_eq!(engine.metrics().rejected_objective_not_improved_total, 1);
}

#[test]
fn assembly_rejects_superior_replacement_when_gas_limit_would_still_be_exceeded() {
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 60_000,
    });

    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0xb1)],
        100,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));
    let _ = engine.insert(candidate(
        "cand-b",
        vec![hash(0xb2)],
        100,
        21_000,
        AssemblyCandidateKind::Transaction,
        true,
    ));

    let decision = engine.insert(candidate(
        "cand-bundle",
        vec![hash(0xb1), hash(0xb2)],
        400,
        80_000,
        AssemblyCandidateKind::Bundle,
        true,
    ));

    assert_eq!(
        decision,
        AssemblyDecision::Rejected {
            candidate_id: "cand-bundle".to_owned(),
            reason: "block_gas_limit_exceeded".to_owned(),
        }
    );
    assert_eq!(engine.snapshot().candidates.len(), 2);
    assert_eq!(engine.metrics().rejected_gas_limit_total, 1);
}

fn candidate(
    candidate_id: &str,
    tx_hashes: Vec<[u8; 32]>,
    priority_score: u32,
    gas_used: u64,
    kind: AssemblyCandidateKind,
    simulation_approved: bool,
) -> AssemblyCandidate {
    AssemblyCandidate {
        candidate_id: candidate_id.to_owned(),
        tx_hashes,
        priority_score,
        gas_used,
        kind,
        simulation: SimulationApproval {
            sim_id: format!("sim-{candidate_id}"),
            block_number: 22_222_222,
            approved: simulation_approved,
        },
    }
}

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}
