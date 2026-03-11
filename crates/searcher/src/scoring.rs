//! Shared scoring primitives used by searcher strategies.

use serde::{Deserialize, Serialize};

const SCORER_VERSION: &str = "scorer.v1";

/// Returns the current scorer version string stamped into candidates.
pub const fn scorer_version() -> &'static str {
    SCORER_VERSION
}

/// Score decomposition that explains how one candidate reached its total score.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub mev_component: u32,
    pub urgency_component: u32,
    pub structural_component: u32,
    pub strategy_bonus: u32,
}

impl ScoreBreakdown {
    /// Returns the fully aggregated score across all components.
    pub fn total(&self) -> u32 {
        self.mev_component
            .saturating_add(self.urgency_component)
            .saturating_add(self.structural_component)
            .saturating_add(self.strategy_bonus)
    }
}
