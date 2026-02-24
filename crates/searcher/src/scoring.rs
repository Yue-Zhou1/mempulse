use serde::{Deserialize, Serialize};

pub const SCORER_VERSION: &str = "scorer.v1";

#[inline]
pub const fn scorer_version() -> &'static str {
    SCORER_VERSION
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub mev_component: u32,
    pub urgency_component: u32,
    pub structural_component: u32,
    pub strategy_bonus: u32,
}

impl ScoreBreakdown {
    pub fn total(&self) -> u32 {
        self.mev_component
            .saturating_add(self.urgency_component)
            .saturating_add(self.structural_component)
            .saturating_add(self.strategy_bonus)
    }
}
