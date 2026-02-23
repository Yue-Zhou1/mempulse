#![forbid(unsafe_code)]

use crate::{OpportunityCandidate, scoring::ScoreBreakdown};
use feature_engine::FeaturedTransaction;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize)]
pub enum StrategyKind {
    SandwichCandidate,
    BackrunCandidate,
    ArbCandidate,
}

pub trait Strategy {
    fn evaluate(&self, featured: &FeaturedTransaction) -> Option<OpportunityCandidate>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SandwichCandidateStrategy;

#[derive(Clone, Copy, Debug, Default)]
pub struct BackrunCandidateStrategy;

#[derive(Clone, Copy, Debug, Default)]
pub struct ArbCandidateStrategy;

impl Strategy for SandwichCandidateStrategy {
    fn evaluate(&self, featured: &FeaturedTransaction) -> Option<OpportunityCandidate> {
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
            format!("calldata/gas structural bonus={}", breakdown.structural_component),
            "strategy bonus=500".to_owned(),
        ];
        Some(candidate(
            StrategyKind::SandwichCandidate,
            featured,
            breakdown,
            reasons,
        ))
    }
}

impl Strategy for BackrunCandidateStrategy {
    fn evaluate(&self, featured: &FeaturedTransaction) -> Option<OpportunityCandidate> {
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
            format!("calldata/gas structural bonus={}", breakdown.structural_component),
            "strategy bonus=300".to_owned(),
        ];
        Some(candidate(
            StrategyKind::BackrunCandidate,
            featured,
            breakdown,
            reasons,
        ))
    }
}

impl Strategy for ArbCandidateStrategy {
    fn evaluate(&self, featured: &FeaturedTransaction) -> Option<OpportunityCandidate> {
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
            format!("calldata/gas structural bonus={}", breakdown.structural_component),
            "strategy bonus=200".to_owned(),
        ];
        Some(candidate(
            StrategyKind::ArbCandidate,
            featured,
            breakdown,
            reasons,
        ))
    }
}

pub fn default_strategies() -> Vec<Box<dyn Strategy + Send + Sync>> {
    vec![
        Box::new(SandwichCandidateStrategy),
        Box::new(BackrunCandidateStrategy),
        Box::new(ArbCandidateStrategy),
    ]
}

fn structural_bonus(featured: &FeaturedTransaction) -> u32 {
    let calldata_bonus = featured.calldata_len.min(512) as u32;
    let gas_bonus = featured.gas_limit.unwrap_or_default().min(500_000) as u32 / 1_000;
    calldata_bonus + gas_bonus
}

fn candidate(
    strategy: StrategyKind,
    featured: &FeaturedTransaction,
    breakdown: ScoreBreakdown,
    reasons: Vec<String>,
) -> OpportunityCandidate {
    OpportunityCandidate {
        tx_hash: featured.hash,
        strategy,
        score: breakdown.total(),
        protocol: featured.analysis.protocol.to_owned(),
        category: featured.analysis.category.to_owned(),
        breakdown,
        reasons,
    }
}
