use builder::{AssemblyCandidate, AssemblyCandidateBuildError, AssemblyCandidateKind};
use searcher::{OpportunityCandidate, OpportunityScoreBreakdown, StrategyKind};
use sim_engine::{ChainContext, SimulationBatchResult, TxSimulationResult};

#[test]
fn materializes_transaction_candidate_from_successful_simulation() {
    let candidate = AssemblyCandidate::from_simulated_opportunity(
        &opportunity(
            hash(0x11),
            vec![hash(0x11)],
            320,
            StrategyKind::SandwichCandidate,
        ),
        &simulation_batch(vec![sim_result(hash(0x11), true, 21_000)]),
    )
    .expect("candidate should materialize");

    assert_eq!(candidate.kind, AssemblyCandidateKind::Transaction);
    assert_eq!(candidate.priority_score, 320);
    assert_eq!(candidate.gas_used, 21_000);
    assert!(candidate.simulation.approved);
    assert_eq!(candidate.tx_hashes, vec![hash(0x11)]);
}

#[test]
fn materializes_bundle_candidate_as_not_approved_when_any_member_fails_simulation() {
    let candidate = AssemblyCandidate::from_simulated_opportunity(
        &opportunity(
            hash(0x21),
            vec![hash(0x21), hash(0x22)],
            540,
            StrategyKind::BundleCandidate,
        ),
        &simulation_batch(vec![
            sim_result(hash(0x21), true, 30_000),
            sim_result(hash(0x22), false, 0),
        ]),
    )
    .expect("candidate should materialize");

    assert_eq!(candidate.kind, AssemblyCandidateKind::Bundle);
    assert_eq!(candidate.gas_used, 30_000);
    assert!(!candidate.simulation.approved);
}

#[test]
fn rejects_candidate_when_simulation_result_for_member_is_missing() {
    let error = AssemblyCandidate::from_simulated_opportunity(
        &opportunity(
            hash(0x31),
            vec![hash(0x31), hash(0x32)],
            420,
            StrategyKind::BundleCandidate,
        ),
        &simulation_batch(vec![sim_result(hash(0x31), true, 30_000)]),
    )
    .expect_err("candidate materialization should fail");

    assert_eq!(
        error,
        AssemblyCandidateBuildError::MissingSimulationResult {
            tx_hash: hash(0x32)
        }
    );
}

fn opportunity(
    tx_hash: [u8; 32],
    member_tx_hashes: Vec<[u8; 32]>,
    score: u32,
    strategy: StrategyKind,
) -> OpportunityCandidate {
    OpportunityCandidate {
        tx_hash,
        member_tx_hashes,
        strategy,
        feature_engine_version: "feature-engine.v1".to_owned(),
        scorer_version: "scorer.v1".to_owned(),
        strategy_version: "strategy.v1".to_owned(),
        score,
        protocol: "uniswap_v3".to_owned(),
        category: "swap".to_owned(),
        breakdown: OpportunityScoreBreakdown {
            mev_component: score,
            urgency_component: 0,
            structural_component: 0,
            strategy_bonus: 0,
        },
        reasons: vec!["test".to_owned()],
    }
}

fn simulation_batch(tx_results: Vec<TxSimulationResult>) -> SimulationBatchResult {
    SimulationBatchResult {
        chain_context: ChainContext {
            chain_id: 1,
            block_number: 22_222_222,
            block_timestamp: 1_700_000_000,
            gas_limit: 30_000_000,
            base_fee_wei: 1_000_000_000,
            coinbase: [0x55; 20],
            state_root: [0xaa; 32],
        },
        tx_results,
        final_state_diff_hash: [0xbb; 32],
    }
}

fn sim_result(hash: [u8; 32], success: bool, gas_used: u64) -> TxSimulationResult {
    TxSimulationResult {
        hash,
        success,
        gas_used,
        state_diff_hash: [0xcc; 32],
        fail_category: (!success).then_some(sim_engine::SimulationFailCategory::Revert),
        trace_id: 7,
    }
}

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}
