#![forbid(unsafe_code)]

use crate::{
    OpportunityCandidate,
    scoring::{ScoreBreakdown, scorer_version},
};
use feature_engine::{FeaturedTransaction, version as feature_engine_version};

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum StrategyKind {
    SandwichCandidate,
    BackrunCandidate,
    ArbCandidate,
    BundleCandidate,
}

const SANDWICH_STRATEGY_VERSION: &str = "strategy.sandwich.v1";
const BACKRUN_STRATEGY_VERSION: &str = "strategy.backrun.v1";
const ARB_STRATEGY_VERSION: &str = "strategy.arb.v1";
const BUNDLE_STRATEGY_VERSION: &str = "strategy.bundle.v1";

pub const fn strategy_version(kind: StrategyKind) -> &'static str {
    match kind {
        StrategyKind::SandwichCandidate => SANDWICH_STRATEGY_VERSION,
        StrategyKind::BackrunCandidate => BACKRUN_STRATEGY_VERSION,
        StrategyKind::ArbCandidate => ARB_STRATEGY_VERSION,
        StrategyKind::BundleCandidate => BUNDLE_STRATEGY_VERSION,
    }
}

pub(crate) fn rank_transaction(
    featured: &FeaturedTransaction,
    min_score: u32,
) -> Vec<OpportunityCandidate> {
    let mut candidates = Vec::with_capacity(3);
    for candidate in [
        evaluate_sandwich_candidate(featured),
        evaluate_backrun_candidate(featured),
        evaluate_arb_candidate(featured),
    ]
    .into_iter()
    .flatten()
    {
        if candidate.score >= min_score {
            candidates.push(candidate);
        }
    }
    candidates
}

fn structural_bonus(featured: &FeaturedTransaction) -> u32 {
    let calldata_bonus = featured.calldata_len.min(512) as u32;
    let gas_bonus = featured.gas_limit.unwrap_or_default().min(500_000) as u32 / 1_000;
    calldata_bonus + gas_bonus
}

fn evaluate_sandwich_candidate(featured: &FeaturedTransaction) -> Option<OpportunityCandidate> {
    let analysis = featured.analysis;
    if analysis.category != "swap" || analysis.mev_score < 70 {
        return None;
    }
    if !analysis.protocol.starts_with("uniswap") && analysis.protocol != "1inch" {
        return None;
    }

    let breakdown = ScoreBreakdown {
        mev_component: analysis.mev_score as u32 * 120,
        urgency_component: analysis.urgency_score as u32 * 25,
        structural_component: structural_bonus(featured),
        strategy_bonus: 500,
    };
    let reasons = vec![
        format!("mev_score={}*120", analysis.mev_score),
        format!("urgency_score={}*25", analysis.urgency_score),
        format!(
            "calldata/gas structural bonus={}",
            breakdown.structural_component
        ),
        "strategy bonus=500".to_owned(),
    ];
    Some(candidate(
        StrategyKind::SandwichCandidate,
        featured,
        breakdown,
        reasons,
    ))
}

fn evaluate_backrun_candidate(featured: &FeaturedTransaction) -> Option<OpportunityCandidate> {
    let analysis = featured.analysis;
    if analysis.category != "swap" || analysis.mev_score < 55 || analysis.urgency_score < 12 {
        return None;
    }

    let breakdown = ScoreBreakdown {
        mev_component: analysis.mev_score as u32 * 100,
        urgency_component: analysis.urgency_score as u32 * 20,
        structural_component: structural_bonus(featured),
        strategy_bonus: 300,
    };
    let reasons = vec![
        format!("mev_score={}*100", analysis.mev_score),
        format!("urgency_score={}*20", analysis.urgency_score),
        format!(
            "calldata/gas structural bonus={}",
            breakdown.structural_component
        ),
        "strategy bonus=300".to_owned(),
    ];
    Some(candidate(
        StrategyKind::BackrunCandidate,
        featured,
        breakdown,
        reasons,
    ))
}

fn evaluate_arb_candidate(featured: &FeaturedTransaction) -> Option<OpportunityCandidate> {
    let analysis = featured.analysis;
    if analysis.category != "swap" || analysis.mev_score < 50 {
        return None;
    }

    let breakdown = ScoreBreakdown {
        mev_component: analysis.mev_score as u32 * 90,
        urgency_component: analysis.urgency_score as u32 * 15,
        structural_component: structural_bonus(featured),
        strategy_bonus: 200,
    };
    let reasons = vec![
        format!("mev_score={}*90", analysis.mev_score),
        format!("urgency_score={}*15", analysis.urgency_score),
        format!(
            "calldata/gas structural bonus={}",
            breakdown.structural_component
        ),
        "strategy bonus=200".to_owned(),
    ];
    Some(candidate(
        StrategyKind::ArbCandidate,
        featured,
        breakdown,
        reasons,
    ))
}

fn candidate(
    strategy: StrategyKind,
    featured: &FeaturedTransaction,
    breakdown: ScoreBreakdown,
    reasons: Vec<String>,
) -> OpportunityCandidate {
    OpportunityCandidate {
        tx_hash: featured.hash,
        member_tx_hashes: vec![featured.hash],
        strategy,
        feature_engine_version: feature_engine_version().to_owned(),
        scorer_version: scorer_version().to_owned(),
        strategy_version: strategy_version(strategy).to_owned(),
        score: breakdown.total(),
        protocol: featured.analysis.protocol.to_owned(),
        category: featured.analysis.category.to_owned(),
        breakdown,
        reasons,
    }
}
