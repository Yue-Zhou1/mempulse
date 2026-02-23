use serde::{Deserialize, Serialize};

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
