use common::{Address, TxHash};
use event_log::TxDecoded;
use searcher::{SearcherConfig, SearcherInputTx, rank_opportunities};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
struct LabelledTx {
    input: SearcherInputTx<'static>,
    is_mev_positive: bool,
    edge_bps: i32,
}

#[derive(Debug, Serialize)]
struct BacktestSummary {
    generated_unix_ms: i64,
    dataset_name: String,
    config: BacktestConfig,
    dataset_size: usize,
    predicted_positive: usize,
    ground_truth_positive: usize,
    confusion: ConfusionMatrix,
    precision: f64,
    recall: f64,
    f1: f64,
    pnl_proxy_bps: i64,
    top_candidates: Vec<CandidateView>,
}

#[derive(Debug, Serialize)]
struct BacktestConfig {
    min_score: u32,
    max_candidates: usize,
}

#[derive(Debug, Serialize)]
struct ConfusionMatrix {
    tp: usize,
    fp: usize,
    fn_: usize,
    tn: usize,
}

#[derive(Debug, Serialize)]
struct CandidateView {
    rank: usize,
    tx_hash: String,
    strategy: String,
    score: u32,
    protocol: String,
    category: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_path = resolve_out_path();
    let dataset = synthetic_dataset();
    let batch = dataset
        .iter()
        .map(|item| item.input.clone())
        .collect::<Vec<_>>();
    let config = SearcherConfig {
        min_score: 8_000,
        max_candidates: 64,
    };
    let ranked = rank_opportunities(&batch, config);

    let truth_positive = dataset
        .iter()
        .filter(|item| item.is_mev_positive)
        .map(|item| item.input.decoded.hash)
        .collect::<BTreeSet<_>>();
    let predicted_positive = ranked
        .iter()
        .map(|candidate| candidate.tx_hash)
        .collect::<BTreeSet<_>>();
    let edge_bps_by_hash = dataset
        .iter()
        .map(|item| (item.input.decoded.hash, item.edge_bps))
        .collect::<BTreeMap<_, _>>();

    let mut tp = 0usize;
    let mut fp = 0usize;
    let mut fn_ = 0usize;
    let mut tn = 0usize;
    for item in &dataset {
        let hash = item.input.decoded.hash;
        let truth = truth_positive.contains(&hash);
        let predicted = predicted_positive.contains(&hash);
        match (truth, predicted) {
            (true, true) => tp = tp.saturating_add(1),
            (false, true) => fp = fp.saturating_add(1),
            (true, false) => fn_ = fn_.saturating_add(1),
            (false, false) => tn = tn.saturating_add(1),
        }
    }

    let precision = ratio(tp, tp.saturating_add(fp));
    let recall = ratio(tp, tp.saturating_add(fn_));
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    let mut pnl_proxy_bps = 0_i64;
    for hash in &predicted_positive {
        if truth_positive.contains(hash) {
            pnl_proxy_bps = pnl_proxy_bps
                .saturating_add((*edge_bps_by_hash.get(hash).unwrap_or(&0)).max(0) as i64);
        } else {
            pnl_proxy_bps = pnl_proxy_bps.saturating_sub(30);
        }
    }

    let top_candidates = ranked
        .iter()
        .take(10)
        .enumerate()
        .map(|(idx, candidate)| CandidateView {
            rank: idx + 1,
            tx_hash: to_hex(candidate.tx_hash),
            strategy: format!("{:?}", candidate.strategy),
            score: candidate.score,
            protocol: candidate.protocol.clone(),
            category: candidate.category.clone(),
        })
        .collect::<Vec<_>>();

    let report = BacktestSummary {
        generated_unix_ms: unix_ms_now(),
        dataset_name: "synthetic-mainnet-like-mix-v1".to_owned(),
        config: BacktestConfig {
            min_score: config.min_score,
            max_candidates: config.max_candidates,
        },
        dataset_size: dataset.len(),
        predicted_positive: predicted_positive.len(),
        ground_truth_positive: truth_positive.len(),
        confusion: ConfusionMatrix { tp, fp, fn_, tn },
        precision,
        recall,
        f1,
        pnl_proxy_bps,
        top_candidates,
    };

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&out_path, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "searcher-backtest: n={} tp={} fp={} fn={} precision={:.4} recall={:.4} f1={:.4} pnl_proxy_bps={} out={}",
        report.dataset_size,
        report.confusion.tp,
        report.confusion.fp,
        report.confusion.fn_,
        report.precision,
        report.recall,
        report.f1,
        report.pnl_proxy_bps,
        out_path.display()
    );
    Ok(())
}

fn resolve_out_path() -> PathBuf {
    let args = env::args().collect::<Vec<_>>();
    let mut idx = 1;
    while idx < args.len() {
        if args[idx] == "--out" && idx + 1 < args.len() {
            return PathBuf::from(&args[idx + 1]);
        }
        idx += 1;
    }
    PathBuf::from("artifacts/searcher/backtest_summary.json")
}

fn synthetic_dataset() -> Vec<LabelledTx> {
    let uniswap_v2 = [
        0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
        0xc6, 0x59, 0xf2, 0x48, 0x8d,
    ];
    let uniswap_v3 = [
        0x68, 0xb3, 0x46, 0x58, 0x33, 0xfb, 0x72, 0xa7, 0x0e, 0xcd, 0xf4, 0x85, 0xe0, 0xe4, 0xc7,
        0xbd, 0x86, 0x65, 0xfc, 0x45,
    ];
    let one_inch = [
        0x11, 0x11, 0x11, 0x12, 0x54, 0xee, 0xb2, 0x54, 0x77, 0xb6, 0x8f, 0xb8, 0x5e, 0xd9, 0x29,
        0xf7, 0x3a, 0x96, 0x05, 0x82,
    ];
    let erc20 = address(0xaa);
    let unknown = address(0x44);

    vec![
        labelled(
            0x10,
            uniswap_v2,
            320_000,
            7_000_000_000,
            256,
            vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4],
            (true, 110),
        ),
        labelled(
            0x11,
            uniswap_v3,
            290_000,
            6_000_000_000,
            212,
            vec![0x04, 0xe4, 0x5a, 0xaf, 9, 8, 7, 6],
            (true, 90),
        ),
        labelled(
            0x12,
            one_inch,
            310_000,
            8_000_000_000,
            244,
            vec![0x50, 0x23, 0xb4, 0xdf, 2, 2, 2, 2],
            (true, 95),
        ),
        labelled(
            0x13,
            uniswap_v2,
            240_000,
            4_000_000_000,
            180,
            vec![0x18, 0xcb, 0xaf, 0xe5, 1, 1, 1, 1],
            (true, 85),
        ),
        labelled(
            0x14,
            uniswap_v3,
            260_000,
            5_000_000_000,
            192,
            vec![0xb8, 0x58, 0x18, 0x3f, 1, 3, 5, 7],
            (true, 80),
        ),
        labelled(
            0x15,
            unknown,
            120_000,
            2_000_000_000,
            96,
            vec![0x38, 0xed, 0x17, 0x39, 0, 0, 0, 0],
            (true, 40),
        ),
        labelled(
            0x30,
            erc20,
            65_000,
            1_000_000_000,
            68,
            vec![0xa9, 0x05, 0x9c, 0xbb, 0, 0, 0, 0],
            (false, 0),
        ),
        labelled(
            0x31,
            erc20,
            68_000,
            1_500_000_000,
            72,
            vec![0x23, 0xb8, 0x72, 0xdd, 0, 0, 0, 0],
            (false, 0),
        ),
        labelled(
            0x32,
            unknown,
            75_000,
            1_000_000_000,
            64,
            vec![0x09, 0x5e, 0xa7, 0xb3, 0, 0, 0, 0],
            (false, 0),
        ),
        labelled(
            0x33,
            unknown,
            90_000,
            1_000_000_000,
            44,
            vec![0xde, 0xad, 0xbe, 0xef, 0, 1, 2, 3],
            (false, 0),
        ),
        labelled(
            0x34,
            erc20,
            60_000,
            800_000_000,
            68,
            vec![0xa9, 0x05, 0x9c, 0xbb, 4, 5, 6, 7],
            (false, 0),
        ),
        labelled(
            0x35,
            unknown,
            100_000,
            500_000_000,
            52,
            vec![0, 0, 0, 0],
            (false, 0),
        ),
    ]
}

fn labelled(
    hash_v: u8,
    to: Address,
    gas_limit: u64,
    priority_fee_wei: u128,
    calldata_len: u32,
    calldata: Vec<u8>,
    label: (bool, i32),
) -> LabelledTx {
    let (is_mev_positive, edge_bps) = label;
    LabelledTx {
        input: SearcherInputTx::owned(
            tx(hash_v, to, gas_limit, priority_fee_wei, calldata_len),
            calldata,
        ),
        is_mev_positive,
        edge_bps,
    }
}

fn tx(
    hash_v: u8,
    to: Address,
    gas_limit: u64,
    priority_fee_wei: u128,
    calldata_len: u32,
) -> TxDecoded {
    TxDecoded {
        hash: hash(hash_v),
        tx_type: 2,
        sender: address(hash_v),
        nonce: hash_v as u64,
        chain_id: Some(1),
        to: Some(to),
        value_wei: Some(1_000_000_000_000_000),
        gas_limit: Some(gas_limit),
        gas_price_wei: None,
        max_fee_per_gas_wei: Some(45_000_000_000),
        max_priority_fee_per_gas_wei: Some(priority_fee_wei),
        max_fee_per_blob_gas_wei: None,
        calldata_len: Some(calldata_len),
    }
}

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

fn to_hex(hash: TxHash) -> String {
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
