#![forbid(unsafe_code)]

mod scoring;
mod strategies;

use common::TxHash;
use event_log::TxDecoded;
use feature_engine::analyze_decoded_transaction;
use scoring::ScoreBreakdown;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use strategies::default_strategies;

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

pub fn rank_opportunities(
    batch: &[SearcherInputTx<'_>],
    config: SearcherConfig,
) -> Vec<OpportunityCandidate> {
    let strategies = default_strategies();
    let mut candidates = Vec::new();

    for input in batch {
        let featured = analyze_decoded_transaction(&input.decoded, input.calldata.as_ref());
        for strategy in &strategies {
            if let Some(candidate) = strategy.evaluate(&featured)
                && candidate.score >= config.min_score
            {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.strategy.cmp(&right.strategy))
            .then_with(|| left.tx_hash.cmp(&right.tx_hash))
    });
    candidates.truncate(config.max_candidates);
    candidates
}
