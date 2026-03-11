#![forbid(unsafe_code)]

use common::{Address, SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use scheduler::{
    PersistedSchedulerSnapshot, SchedulerCandidate, SchedulerConfig, SchedulerSimulationResult,
    ValidatedTransaction, scheduler_channel,
};
use searcher::{SearcherConfig, SearcherInputTx, rank_opportunity_batch};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use storage::{EventStore, InMemoryStorage};
use tokio::runtime::{Builder, Runtime};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LatencySummary {
    pub iterations: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub avg_us: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PipelineLatencyReport {
    pub batch_size: usize,
    pub iterations: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub avg_us: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerPipelineOutcome {
    pub ready_total: usize,
    pub pending_total: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerPipelineReport {
    pub batch_size: usize,
    pub ready_total: usize,
    pub pending_total: usize,
    #[serde(flatten)]
    pub latency: LatencySummary,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationRoundtripOutcome {
    pub simulation_tasks: usize,
    pub handoff_total: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationRoundtripReport {
    pub candidate_count: usize,
    pub simulation_tasks: usize,
    pub handoff_total: usize,
    #[serde(flatten)]
    pub latency: LatencySummary,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageSnapshotOutcome {
    pub snapshot_present: bool,
    pub replay_event_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageSnapshotReport {
    pub pending_count: usize,
    pub tail_event_count: usize,
    pub replay_event_count: usize,
    pub snapshot_present: bool,
    pub iterations: usize,
    pub write_p50_us: u64,
    pub write_p95_us: u64,
    pub write_p99_us: u64,
    pub write_avg_us: u64,
    pub rehydrate_p50_us: u64,
    pub rehydrate_p95_us: u64,
    pub rehydrate_p99_us: u64,
    pub rehydrate_avg_us: u64,
}

pub fn synthetic_batch(batch_size: usize) -> Vec<SearcherInputTx<'static>> {
    let routers = [
        uniswap_v2_router(),
        uniswap_v3_router(),
        [0x11; 20],
        [0x22; 20],
    ];
    let selectors = [
        [0x38, 0xed, 0x17, 0x39],
        [0x04, 0xe4, 0x5a, 0xaf],
        [0x50, 0x23, 0xb4, 0xdf],
        [0xa9, 0x05, 0x9c, 0xbb],
    ];

    (0..batch_size)
        .map(|idx| {
            let router = routers[idx % routers.len()];
            let selector = selectors[idx % selectors.len()];
            let calldata_len = 32 + ((idx % 6) * 32);
            SearcherInputTx::owned(
                TxDecoded {
                    hash: tx_hash(idx as u64 + 1),
                    tx_type: 2,
                    sender: address((idx % 251) as u8),
                    nonce: idx as u64,
                    chain_id: Some(1),
                    to: Some(router),
                    value_wei: Some(1_000_000_000_000_000),
                    gas_limit: Some(180_000 + ((idx % 5) as u64) * 30_000),
                    gas_price_wei: None,
                    max_fee_per_gas_wei: Some(45_000_000_000 + ((idx % 7) as u128) * 1_000_000_000),
                    max_priority_fee_per_gas_wei: Some(
                        1_000_000_000 + ((idx % 5) as u128) * 1_000_000_000,
                    ),
                    max_fee_per_blob_gas_wei: None,
                    calldata_len: Some(calldata_len as u32),
                },
                build_calldata(selector, calldata_len),
            )
        })
        .collect()
}

pub fn run_pipeline_once(batch: &[SearcherInputTx<'_>]) -> usize {
    let config = SearcherConfig {
        min_score: 7_500,
        max_candidates: 128,
    };
    rank_opportunity_batch(batch, config).candidates.len()
}

pub fn measure_pipeline_latency(batch_size: usize, iterations: usize) -> PipelineLatencyReport {
    let iterations = iterations.max(5);
    let batch = synthetic_batch(batch_size.max(1));
    let mut samples = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let start = Instant::now();
        let _ = run_pipeline_once(&batch);
        samples.push(start.elapsed().as_micros() as u64);
    }

    let latency = summarize_latency_samples(&samples);
    PipelineLatencyReport {
        batch_size: batch.len(),
        iterations: latency.iterations,
        p50_us: latency.p50_us,
        p95_us: latency.p95_us,
        p99_us: latency.p99_us,
        avg_us: latency.avg_us,
    }
}

pub fn run_scheduler_pipeline_once(batch_size: usize) -> SchedulerPipelineOutcome {
    let runtime = build_tokio_runtime();
    let transactions = synthetic_validated_transactions(batch_size.max(1));
    runtime.block_on(async { scheduler_pipeline_iteration(&transactions).await })
}

pub fn measure_scheduler_pipeline_latency(
    batch_size: usize,
    iterations: usize,
) -> SchedulerPipelineReport {
    let iterations = iterations.max(5);
    let runtime = build_tokio_runtime();
    let transactions = synthetic_validated_transactions(batch_size.max(1));
    let mut samples = Vec::with_capacity(iterations);
    let mut last_outcome = SchedulerPipelineOutcome::default();

    for _ in 0..iterations {
        let start = Instant::now();
        last_outcome =
            runtime.block_on(async { scheduler_pipeline_iteration(&transactions).await });
        samples.push(start.elapsed().as_micros() as u64);
    }

    SchedulerPipelineReport {
        batch_size: transactions.len(),
        ready_total: last_outcome.ready_total,
        pending_total: last_outcome.pending_total,
        latency: summarize_latency_samples(&samples),
    }
}

pub fn run_simulation_roundtrip_once(candidate_count: usize) -> SimulationRoundtripOutcome {
    let runtime = build_tokio_runtime();
    let transactions = synthetic_validated_transactions(candidate_count.max(1));
    runtime.block_on(async { simulation_roundtrip_iteration(&transactions).await })
}

pub fn measure_simulation_roundtrip_latency(
    candidate_count: usize,
    iterations: usize,
) -> SimulationRoundtripReport {
    let iterations = iterations.max(5);
    let runtime = build_tokio_runtime();
    let transactions = synthetic_validated_transactions(candidate_count.max(1));
    let mut samples = Vec::with_capacity(iterations);
    let mut last_outcome = SimulationRoundtripOutcome::default();

    for _ in 0..iterations {
        let start = Instant::now();
        last_outcome =
            runtime.block_on(async { simulation_roundtrip_iteration(&transactions).await });
        samples.push(start.elapsed().as_micros() as u64);
    }

    SimulationRoundtripReport {
        candidate_count: transactions.len(),
        simulation_tasks: last_outcome.simulation_tasks,
        handoff_total: last_outcome.handoff_total,
        latency: summarize_latency_samples(&samples),
    }
}

pub fn run_storage_snapshot_once(
    pending_count: usize,
    tail_event_count: usize,
) -> StorageSnapshotOutcome {
    let pending_count = pending_count.max(1);
    let tail_event_count = tail_event_count.max(1);
    let transactions = synthetic_validated_transactions(pending_count);
    let snapshot = synthetic_scheduler_snapshot(&transactions);
    let template = synthetic_storage_template(&transactions);
    let tail_events = synthetic_tail_events(transactions.len() as u64, tail_event_count);

    let (_, _, outcome) = storage_snapshot_iteration(template, snapshot, &tail_events);
    outcome
}

pub fn measure_storage_snapshot_latency(
    pending_count: usize,
    tail_event_count: usize,
    iterations: usize,
) -> StorageSnapshotReport {
    let pending_count = pending_count.max(1);
    let tail_event_count = tail_event_count.max(1);
    let iterations = iterations.max(5);
    let transactions = synthetic_validated_transactions(pending_count);
    let snapshot = synthetic_scheduler_snapshot(&transactions);
    let template = synthetic_storage_template(&transactions);
    let tail_events = synthetic_tail_events(transactions.len() as u64, tail_event_count);
    let mut write_samples = Vec::with_capacity(iterations);
    let mut rehydrate_samples = Vec::with_capacity(iterations);
    let mut last_outcome = StorageSnapshotOutcome::default();

    for _ in 0..iterations {
        let (write_us, rehydrate_us, outcome) =
            storage_snapshot_iteration(template.clone(), snapshot.clone(), &tail_events);
        write_samples.push(write_us);
        rehydrate_samples.push(rehydrate_us);
        last_outcome = outcome;
    }

    let write_latency = summarize_latency_samples(&write_samples);
    let rehydrate_latency = summarize_latency_samples(&rehydrate_samples);
    StorageSnapshotReport {
        pending_count: transactions.len(),
        tail_event_count: tail_events.len(),
        replay_event_count: last_outcome.replay_event_count,
        snapshot_present: last_outcome.snapshot_present,
        iterations,
        write_p50_us: write_latency.p50_us,
        write_p95_us: write_latency.p95_us,
        write_p99_us: write_latency.p99_us,
        write_avg_us: write_latency.avg_us,
        rehydrate_p50_us: rehydrate_latency.p50_us,
        rehydrate_p95_us: rehydrate_latency.p95_us,
        rehydrate_p99_us: rehydrate_latency.p99_us,
        rehydrate_avg_us: rehydrate_latency.avg_us,
    }
}

pub fn summarize_latency_samples(samples: &[u64]) -> LatencySummary {
    if samples.is_empty() {
        return LatencySummary::default();
    }

    let mut sorted_samples = samples.to_vec();
    sorted_samples.sort_unstable();
    let total: u128 = sorted_samples.iter().copied().map(u128::from).sum();

    LatencySummary {
        iterations: sorted_samples.len(),
        p50_us: percentile(&sorted_samples, 0.50),
        p95_us: percentile(&sorted_samples, 0.95),
        p99_us: percentile(&sorted_samples, 0.99),
        avg_us: (total / sorted_samples.len() as u128) as u64,
    }
}

async fn scheduler_pipeline_iteration(
    transactions: &[ValidatedTransaction],
) -> SchedulerPipelineOutcome {
    let (handle, runtime) =
        scheduler_channel(SchedulerConfig::default()).expect("valid scheduler config");
    let runtime_task = tokio::spawn(runtime.run());

    for transaction in transactions.iter().cloned() {
        handle.admit(transaction).await.expect("admit transaction");
    }

    let snapshot = handle.snapshot();
    runtime_task.abort();
    let _ = runtime_task.await;

    SchedulerPipelineOutcome {
        ready_total: snapshot.ready.len(),
        pending_total: snapshot.pending.len(),
    }
}

async fn simulation_roundtrip_iteration(
    transactions: &[ValidatedTransaction],
) -> SimulationRoundtripOutcome {
    let (handle, runtime) =
        scheduler_channel(SchedulerConfig::default()).expect("valid scheduler config");
    let runtime_task = tokio::spawn(runtime.run());

    for transaction in transactions.iter().cloned() {
        handle.admit(transaction).await.expect("admit transaction");
    }

    let candidates = transactions
        .iter()
        .enumerate()
        .map(|(idx, transaction)| SchedulerCandidate {
            candidate_id: format!("cand-{idx}").into(),
            tx_hash: transaction.hash(),
            member_tx_hashes: vec![transaction.hash()],
            score: 10_000 + idx as u32,
            strategy: "SandwichCandidate".into(),
            detected_unix_ms: transaction.observed_at_unix_ms,
        })
        .collect::<Vec<_>>();

    let dispatch = handle
        .register_candidates(candidates)
        .await
        .expect("register candidates");

    let mut handoff_total = 0usize;
    let simulation_tasks = dispatch.simulation_tasks.len();
    for task in dispatch.simulation_tasks {
        let outcome = handle
            .apply_simulation_result(SchedulerSimulationResult {
                candidate_id: task.candidate_id.clone(),
                tx_hash: task.tx_hash,
                member_tx_hashes: task.member_tx_hashes.clone(),
                block_number: task.block_number,
                generation: task.generation,
                approved: true,
            })
            .await
            .expect("apply simulation result");
        handoff_total += outcome.builder_handoffs.len();
    }

    runtime_task.abort();
    let _ = runtime_task.await;

    SimulationRoundtripOutcome {
        simulation_tasks,
        handoff_total,
    }
}

fn storage_snapshot_iteration(
    mut storage: InMemoryStorage,
    snapshot: PersistedSchedulerSnapshot,
    tail_events: &[EventEnvelope],
) -> (u64, u64, StorageSnapshotOutcome) {
    let write_start = Instant::now();
    storage.write_scheduler_snapshot(snapshot);
    let write_us = write_start.elapsed().as_micros() as u64;

    for event in tail_events {
        storage.append_event(event.clone());
    }

    let rehydrate_start = Instant::now();
    let plan = storage.scheduler_rehydration_plan(60_000);
    let rehydrate_us = rehydrate_start.elapsed().as_micros() as u64;

    (
        write_us,
        rehydrate_us,
        StorageSnapshotOutcome {
            snapshot_present: plan.snapshot.is_some(),
            replay_event_count: plan.replay_events.len(),
        },
    )
}

fn build_tokio_runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build bench tokio runtime")
}

fn synthetic_validated_transactions(batch_size: usize) -> Vec<ValidatedTransaction> {
    (0..batch_size)
        .map(|idx| {
            let hash_seed = idx as u64 + 1;
            ValidatedTransaction {
                source_id: SourceId::new("bench"),
                observed_at_unix_ms: 1_700_000_000_000 + idx as i64,
                observed_at_mono_ns: (idx as u64 + 1) * 1_000,
                calldata: build_calldata([0x38, 0xed, 0x17, 0x39], 36),
                decoded: TxDecoded {
                    hash: tx_hash(hash_seed),
                    tx_type: 2,
                    sender: address_from_u64(hash_seed),
                    nonce: idx as u64,
                    chain_id: Some(1),
                    to: Some(uniswap_v2_router()),
                    value_wei: Some(1_000_000_000_000_000),
                    gas_limit: Some(180_000),
                    gas_price_wei: None,
                    max_fee_per_gas_wei: Some(45_000_000_000),
                    max_priority_fee_per_gas_wei: Some(2_000_000_000),
                    max_fee_per_blob_gas_wei: None,
                    calldata_len: Some(36),
                },
            }
        })
        .collect()
}

fn synthetic_scheduler_snapshot(
    transactions: &[ValidatedTransaction],
) -> PersistedSchedulerSnapshot {
    let runtime = build_tokio_runtime();
    runtime.block_on(async {
        let (handle, runtime) =
            scheduler_channel(SchedulerConfig::default()).expect("valid scheduler config");
        let runtime_task = tokio::spawn(runtime.run());

        for transaction in transactions.iter().cloned() {
            handle.admit(transaction).await.expect("admit transaction");
        }

        let mut snapshot = handle.persisted_snapshot(1_700_000_000_500, 500);
        // The synthetic storage template preloads one decoded event per pending
        // transaction, so the persisted snapshot must point at that prefix.
        snapshot.event_seq_hi = transactions.len() as u64;
        runtime_task.abort();
        let _ = runtime_task.await;
        snapshot
    })
}

fn synthetic_storage_template(transactions: &[ValidatedTransaction]) -> InMemoryStorage {
    let mut storage = InMemoryStorage::default();
    for (idx, transaction) in transactions.iter().enumerate() {
        storage.append_event(decoded_event(idx as u64 + 1, transaction));
    }
    storage
}

fn synthetic_tail_events(start_seq_id: u64, tail_event_count: usize) -> Vec<EventEnvelope> {
    synthetic_validated_transactions(tail_event_count)
        .into_iter()
        .enumerate()
        .map(|(idx, transaction)| decoded_event(start_seq_id + idx as u64 + 1, &transaction))
        .collect()
}

fn decoded_event(seq_id: u64, transaction: &ValidatedTransaction) -> EventEnvelope {
    EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: transaction.observed_at_unix_ms,
        ingest_ts_mono_ns: transaction.observed_at_mono_ns,
        source_id: transaction.source_id.clone(),
        payload: EventPayload::TxDecoded(transaction.decoded.clone()),
    }
}

fn build_calldata(selector: [u8; 4], calldata_len: usize) -> Vec<u8> {
    let mut calldata = Vec::with_capacity(calldata_len.max(4));
    calldata.extend_from_slice(&selector);
    while calldata.len() < calldata_len {
        calldata.push((calldata.len() % 251) as u8);
    }
    calldata
}

fn percentile(sorted_samples: &[u64], q: f64) -> u64 {
    if sorted_samples.is_empty() {
        return 0;
    }
    let q = q.clamp(0.0, 1.0);
    let max_index = sorted_samples.len().saturating_sub(1);
    let index = ((max_index as f64) * q).round() as usize;
    sorted_samples[index.min(max_index)]
}

fn tx_hash(value: u64) -> TxHash {
    let mut hash = [0_u8; 32];
    hash[..8].copy_from_slice(&value.to_be_bytes());
    hash
}

fn address(value: u8) -> Address {
    [value; 20]
}

fn address_from_u64(value: u64) -> Address {
    let mut out = [0_u8; 20];
    out[12..].copy_from_slice(&value.to_be_bytes());
    out
}

fn uniswap_v2_router() -> Address {
    [
        0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
        0xc6, 0x59, 0xf2, 0x48, 0x8d,
    ]
}

fn uniswap_v3_router() -> Address {
    [
        0x68, 0xb3, 0x46, 0x58, 0x33, 0xfb, 0x72, 0xa7, 0x0e, 0xcd, 0xf4, 0x85, 0xe0, 0xe4, 0xc7,
        0xbd, 0x86, 0x65, 0xfc, 0x45,
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        measure_pipeline_latency, measure_scheduler_pipeline_latency,
        measure_simulation_roundtrip_latency, measure_storage_snapshot_latency, run_pipeline_once,
        run_scheduler_pipeline_once, run_simulation_roundtrip_once, run_storage_snapshot_once,
        synthetic_batch,
    };

    #[test]
    fn pipeline_latency_snapshot() {
        let report = measure_pipeline_latency(256, 40);
        println!("PIPELINE_BATCH_SIZE={}", report.batch_size);
        println!("PIPELINE_ITERATIONS={}", report.iterations);
        println!("PIPELINE_P50_US={}", report.p50_us);
        println!("PIPELINE_P95_US={}", report.p95_us);
        println!("PIPELINE_P99_US={}", report.p99_us);
        println!("PIPELINE_AVG_US={}", report.avg_us);

        assert_eq!(report.batch_size, 256);
        assert!(report.p95_us >= report.p50_us);
    }

    #[test]
    fn pipeline_executes_and_returns_candidates() {
        let batch = synthetic_batch(128);
        let count = run_pipeline_once(&batch);
        assert!(count > 0);
    }

    #[test]
    fn scheduler_pipeline_executes_and_returns_ready_frontier() {
        let outcome = run_scheduler_pipeline_once(64);
        assert_eq!(outcome.ready_total, 64);
        assert_eq!(outcome.pending_total, 64);

        let report = measure_scheduler_pipeline_latency(64, 10);
        assert_eq!(report.ready_total, 64);
        assert_eq!(report.pending_total, 64);
        assert!(report.latency.p95_us >= report.latency.p50_us);
    }

    #[test]
    fn simulation_roundtrip_executes_and_returns_builder_handoffs() {
        let outcome = run_simulation_roundtrip_once(32);
        assert_eq!(outcome.simulation_tasks, 32);
        assert_eq!(outcome.handoff_total, 32);

        let report = measure_simulation_roundtrip_latency(32, 10);
        assert_eq!(report.simulation_tasks, 32);
        assert_eq!(report.handoff_total, 32);
        assert!(report.latency.p95_us >= report.latency.p50_us);
    }

    #[test]
    fn storage_snapshot_executes_and_returns_tail_replay() {
        let outcome = run_storage_snapshot_once(64, 8);
        assert!(outcome.snapshot_present);
        assert_eq!(outcome.replay_event_count, 8);

        let report = measure_storage_snapshot_latency(64, 8, 10);
        assert!(report.snapshot_present);
        assert_eq!(report.replay_event_count, 8);
        assert!(report.write_p95_us >= report.write_p50_us);
        assert!(report.rehydrate_p95_us >= report.rehydrate_p50_us);
    }
}
