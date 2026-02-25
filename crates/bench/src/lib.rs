#![forbid(unsafe_code)]

use common::{Address, TxHash};
use event_log::TxDecoded;
use searcher::{SearcherConfig, SearcherInputTx, rank_opportunities};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PipelineLatencyReport {
    pub batch_size: usize,
    pub iterations: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub avg_us: u64,
}

pub fn synthetic_batch(batch_size: usize) -> Vec<SearcherInputTx<'static>> {
    let routers = [
        uniswap_v2_router(),
        uniswap_v3_router(),
        [0x11; 20],
        [0x22; 20],
    ];
    let selectors = [
        [0x38, 0xed, 0x17, 0x39], // uniswap v2 swap
        [0x04, 0xe4, 0x5a, 0xaf], // uniswap v3 exactInputSingle
        [0x50, 0x23, 0xb4, 0xdf], // uniswap v3 exactOutputSingle
        [0xa9, 0x05, 0x9c, 0xbb], // erc20 transfer
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
    rank_opportunities(batch, config).len()
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

    samples.sort_unstable();
    let total: u128 = samples.iter().copied().map(u128::from).sum();
    let avg = (total / samples.len() as u128) as u64;

    PipelineLatencyReport {
        batch_size: batch.len(),
        iterations,
        p50_us: percentile(&samples, 0.50),
        p95_us: percentile(&samples, 0.95),
        p99_us: percentile(&samples, 0.99),
        avg_us: avg,
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
    use super::{measure_pipeline_latency, run_pipeline_once, synthetic_batch};

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
}
