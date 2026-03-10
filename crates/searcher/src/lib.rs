#![forbid(unsafe_code)]

mod scoring;
mod strategies;

use common::{Address, TxHash};
use event_log::TxDecoded;
use feature_engine::{analyze_decoded_transaction, version as feature_engine_version};
use scoring::ScoreBreakdown;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

pub use scoring::ScoreBreakdown as OpportunityScoreBreakdown;
pub use scoring::scorer_version;
pub use strategies::{StrategyKind, strategy_version};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearcherInputTx<'a> {
    pub decoded: TxDecoded,
    pub calldata: Cow<'a, [u8]>,
}

pub type OwnedSearcherInputTx = SearcherInputTx<'static>;

impl<'a> SearcherInputTx<'a> {
    pub fn borrowed(decoded: TxDecoded, calldata: &'a [u8]) -> Self {
        Self {
            decoded,
            calldata: Cow::Borrowed(calldata),
        }
    }

    pub fn owned(decoded: TxDecoded, calldata: Vec<u8>) -> OwnedSearcherInputTx {
        OwnedSearcherInputTx {
            decoded,
            calldata: Cow::Owned(calldata),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearcherConfig {
    pub min_score: u32,
    pub max_candidates: usize,
}

impl Default for SearcherConfig {
    fn default() -> Self {
        Self {
            min_score: 0,
            max_candidates: 64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpportunityCandidate {
    pub tx_hash: TxHash,
    pub member_tx_hashes: Vec<TxHash>,
    pub strategy: StrategyKind,
    pub feature_engine_version: String,
    pub scorer_version: String,
    pub strategy_version: String,
    pub score: u32,
    pub protocol: String,
    pub category: String,
    pub breakdown: ScoreBreakdown,
    pub reasons: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearcherBatchMetrics {
    pub input_transactions: usize,
    pub generated_candidates: usize,
    pub returned_candidates: usize,
    pub truncated_candidates: usize,
    pub bundle_candidates_generated: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearcherBatchResult {
    pub candidates: Vec<OpportunityCandidate>,
    pub metrics: SearcherBatchMetrics,
}

pub fn rank_opportunity_batch(
    batch: &[SearcherInputTx<'_>],
    config: SearcherConfig,
) -> SearcherBatchResult {
    let mut candidates = Vec::new();
    let mut bundle_inputs = Vec::new();

    for input in batch {
        let featured = analyze_decoded_transaction(&input.decoded, input.calldata.as_ref());
        let mut per_tx_candidates = strategies::rank_transaction(&featured, config.min_score);
        per_tx_candidates.sort_by(candidate_sort_key);
        if let Some(top_candidate) = per_tx_candidates.first().cloned() {
            bundle_inputs.push(BundleInput {
                sender: input.decoded.sender,
                nonce: input.decoded.nonce,
                candidate: top_candidate,
            });
        }
        candidates.extend(per_tx_candidates);
    }

    let bundle_candidates = build_bundle_candidates(&bundle_inputs);
    let bundle_candidates_generated = bundle_candidates.len();
    candidates.extend(bundle_candidates);

    candidates.sort_by(candidate_sort_key);
    let generated_candidates = candidates.len();
    candidates.truncate(config.max_candidates);
    SearcherBatchResult {
        metrics: SearcherBatchMetrics {
            input_transactions: batch.len(),
            generated_candidates,
            returned_candidates: candidates.len(),
            truncated_candidates: generated_candidates.saturating_sub(candidates.len()),
            bundle_candidates_generated,
        },
        candidates,
    }
}

#[derive(Clone, Debug)]
struct BundleInput {
    sender: Address,
    nonce: u64,
    candidate: OpportunityCandidate,
}

fn build_bundle_candidates(inputs: &[BundleInput]) -> Vec<OpportunityCandidate> {
    let mut sorted = inputs.to_vec();
    sorted.sort_by(|left, right| {
        left.sender
            .cmp(&right.sender)
            .then_with(|| left.nonce.cmp(&right.nonce))
            .then_with(|| candidate_sort_key(&left.candidate, &right.candidate))
    });

    let mut bundles = Vec::new();
    for pair in sorted.windows(2) {
        let [left, right] = pair else {
            continue;
        };
        if left.sender != right.sender
            || right.nonce != left.nonce.saturating_add(1)
            || left.candidate.protocol != right.candidate.protocol
            || left.candidate.category != right.candidate.category
        {
            continue;
        }

        let breakdown = ScoreBreakdown {
            mev_component: left
                .candidate
                .breakdown
                .mev_component
                .saturating_add(right.candidate.breakdown.mev_component),
            urgency_component: left
                .candidate
                .breakdown
                .urgency_component
                .saturating_add(right.candidate.breakdown.urgency_component),
            structural_component: left
                .candidate
                .breakdown
                .structural_component
                .saturating_add(right.candidate.breakdown.structural_component),
            strategy_bonus: left
                .candidate
                .breakdown
                .strategy_bonus
                .saturating_add(right.candidate.breakdown.strategy_bonus)
                .saturating_add(400),
        };
        let score = breakdown.total();
        bundles.push(OpportunityCandidate {
            // Bundle records are keyed by the first member hash in storage; full membership
            // remains available via `member_tx_hashes`.
            tx_hash: left.candidate.tx_hash,
            member_tx_hashes: vec![left.candidate.tx_hash, right.candidate.tx_hash],
            strategy: StrategyKind::BundleCandidate,
            feature_engine_version: feature_engine_version().to_owned(),
            scorer_version: scorer_version().to_owned(),
            strategy_version: strategy_version(StrategyKind::BundleCandidate).to_owned(),
            score,
            protocol: left.candidate.protocol.clone(),
            category: left.candidate.category.clone(),
            breakdown,
            reasons: vec![
                "bundle_size=2".to_owned(),
                format!("contiguous_nonces={}..{}", left.nonce, right.nonce),
                format!(
                    "member_hashes={:?}+{:?}",
                    left.candidate.tx_hash, right.candidate.tx_hash
                ),
                format!(
                    "member_scores={}+{}",
                    left.candidate.score, right.candidate.score
                ),
                "bundle bonus=400".to_owned(),
            ],
        });
    }

    bundles
}

fn candidate_sort_key(
    left: &OpportunityCandidate,
    right: &OpportunityCandidate,
) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.strategy.cmp(&right.strategy))
        .then_with(|| left.tx_hash.cmp(&right.tx_hash))
}
