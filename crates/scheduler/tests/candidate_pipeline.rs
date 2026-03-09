use common::TxHash;
use scheduler::{
    SchedulerCandidate, SchedulerConfig, SchedulerSimulationResult, scheduler_channel,
};

fn sample_candidate(
    candidate_id: &str,
    tx_hash: TxHash,
    member_tx_hashes: Vec<TxHash>,
) -> SchedulerCandidate {
    SchedulerCandidate {
        candidate_id: candidate_id.to_owned(),
        tx_hash,
        member_tx_hashes,
        score: 12_345,
        strategy: "SandwichCandidate".to_owned(),
        detected_unix_ms: 1_700_000_000_000,
    }
}

fn sample_sim_result(
    task: &scheduler::SimulationTaskSpec,
    approved: bool,
) -> SchedulerSimulationResult {
    SchedulerSimulationResult {
        candidate_id: task.candidate_id.clone(),
        tx_hash: task.tx_hash,
        member_tx_hashes: task.member_tx_hashes.clone(),
        block_number: task.block_number,
        generation: task.generation,
        approved,
    }
}

#[tokio::test]
async fn scheduler_registers_candidates_and_issues_simulation_tasks() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let candidate = sample_candidate("cand-1", [0x11; 32], vec![[0x11; 32]]);
    let dispatch = handle
        .register_candidates(vec![candidate.clone()])
        .await
        .expect("register candidates");

    assert_eq!(dispatch.simulation_tasks.len(), 1);
    assert_eq!(dispatch.simulation_tasks[0].candidate_id, "cand-1");

    runtime_task.abort();
}

#[tokio::test]
async fn scheduler_discards_stale_simulation_results_after_head_advance() {
    let (handle, runtime) = scheduler_channel(SchedulerConfig::default());
    let runtime_task = tokio::spawn(runtime.run());

    let candidate = sample_candidate("cand-1", [0x11; 32], vec![[0x11; 32]]);
    let dispatch = handle
        .register_candidates(vec![candidate])
        .await
        .expect("dispatch");
    handle.advance_head(43).await.expect("advance head");
    let applied = handle
        .apply_simulation_result(sample_sim_result(&dispatch.simulation_tasks[0], true))
        .await
        .expect("apply result");

    assert!(applied.builder_handoffs.is_empty());
    assert_eq!(applied.stale_result_drop_total, 1);

    runtime_task.abort();
}
