use anyhow::Result;
use common::TxHash;
use searcher::{OpportunityCandidate, StrategyKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sim_engine::SimulationBatchResult;
use std::time::{Duration, Instant};

use crate::{BlockTemplate, RelayClient, RelayDryRunResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssemblyCandidateKind {
    Transaction,
    Bundle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationApproval {
    pub sim_id: String,
    // This is currently audit metadata only. Runtime callers remain responsible for clearing
    // stale candidates across head changes, so mixed block numbers can coexist in one engine.
    pub block_number: u64,
    pub approved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssemblyCandidate {
    pub candidate_id: String,
    pub tx_hashes: Vec<TxHash>,
    pub priority_score: u32,
    pub gas_used: u64,
    pub kind: AssemblyCandidateKind,
    pub simulation: SimulationApproval,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssemblyCandidateBuildError {
    MissingSimulationResult { tx_hash: TxHash },
}

impl AssemblyCandidate {
    pub fn from_simulated_opportunity(
        opportunity: &OpportunityCandidate,
        simulation: &SimulationBatchResult,
    ) -> Result<Self, AssemblyCandidateBuildError> {
        let tx_hashes = if opportunity.member_tx_hashes.is_empty() {
            vec![opportunity.tx_hash]
        } else {
            opportunity.member_tx_hashes.clone()
        };
        let mut approved = true;
        let mut gas_used = 0_u64;

        for tx_hash in &tx_hashes {
            let Some(result) = simulation
                .tx_results
                .iter()
                .find(|result| result.hash == *tx_hash)
            else {
                return Err(AssemblyCandidateBuildError::MissingSimulationResult {
                    tx_hash: *tx_hash,
                });
            };
            approved &= result.success;
            gas_used = gas_used.saturating_add(result.gas_used);
        }

        Ok(Self {
            candidate_id: format!(
                "{:?}:0x{}",
                opportunity.strategy,
                hex::encode(opportunity.tx_hash)
            ),
            tx_hashes,
            priority_score: opportunity.score,
            gas_used,
            kind: assembly_kind(opportunity.strategy, opportunity.member_tx_hashes.len()),
            simulation: SimulationApproval {
                sim_id: format!("0x{}", hex::encode(simulation.final_state_diff_hash)),
                block_number: simulation.chain_context.block_number,
                approved,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssemblyConfig {
    pub block_gas_limit: u64,
}

impl Default for AssemblyConfig {
    fn default() -> Self {
        Self {
            block_gas_limit: 30_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssemblyObjective {
    pub total_priority_score: u64,
    pub total_gas_used: u64,
    pub total_candidates: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssemblySnapshot {
    pub candidates: Vec<AssemblyCandidate>,
    pub objective: AssemblyObjective,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssemblyMetrics {
    pub inserted_total: u64,
    pub replaced_total: u64,
    pub rejected_total: u64,
    pub rejected_simulation_not_approved_total: u64,
    pub rejected_objective_not_improved_total: u64,
    pub rejected_gas_limit_total: u64,
    pub rollback_total: u64,
    pub active_candidate_total: usize,
    pub total_priority_score: u64,
    pub total_gas_used: u64,
    pub last_decision_latency_ns: u64,
    pub max_decision_latency_ns: u64,
    pub total_decision_latency_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayBuildContext {
    pub slot: u64,
    pub parent_hash: String,
    pub builder_pubkey: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssemblyDecision {
    Inserted {
        candidate_id: String,
        replaced_candidate_ids: Vec<String>,
    },
    Rejected {
        candidate_id: String,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssemblyRollbackDecision {
    RolledBack { candidate_id: String },
    NotFound { candidate_id: String },
}

// The engine keeps an incremental block candidate set but does not automatically evict stale
// entries across block boundaries. Runtime callers should reset or roll back stale candidates
// when the target block changes.
#[derive(Clone, Debug, Default)]
pub struct AssemblyEngine {
    config: AssemblyConfig,
    candidates: Vec<AssemblyCandidate>,
    metrics: AssemblyMetrics,
}

impl AssemblyEngine {
    pub fn new(config: AssemblyConfig) -> Self {
        Self {
            config,
            candidates: Vec::new(),
            metrics: AssemblyMetrics::default(),
        }
    }

    pub fn insert(&mut self, candidate: AssemblyCandidate) -> AssemblyDecision {
        let started_at = Instant::now();
        let decision = self.insert_inner(candidate);
        self.record_decision_latency(started_at.elapsed());
        decision
    }

    pub fn remove_candidate(&mut self, candidate_id: &str) -> AssemblyRollbackDecision {
        let started_at = Instant::now();
        let decision = self.remove_candidate_inner(candidate_id);
        self.record_decision_latency(started_at.elapsed());
        decision
    }

    pub fn snapshot(&self) -> AssemblySnapshot {
        AssemblySnapshot {
            candidates: self.candidates.clone(),
            objective: self.live_objective(),
        }
    }

    pub fn metrics(&self) -> AssemblyMetrics {
        self.metrics
    }

    pub fn build_relay_template(&self, context: &RelayBuildContext) -> Option<BlockTemplate> {
        if self.candidates.is_empty() {
            return None;
        }

        let objective = self.live_objective();
        let tx_count = self.candidates.iter().fold(0_u32, |acc, candidate| {
            acc.saturating_add(candidate.tx_hashes.len() as u32)
        });
        Some(BlockTemplate {
            slot: context.slot,
            parent_hash: context.parent_hash.clone(),
            block_hash: compute_block_hash(context, &self.candidates),
            builder_pubkey: context.builder_pubkey.clone(),
            tx_count,
            gas_used: objective.total_gas_used,
        })
    }

    pub async fn submit_relay_dry_run(
        &self,
        context: &RelayBuildContext,
        relay_client: &RelayClient,
    ) -> Result<Option<RelayDryRunResult>> {
        let Some(template) = self.build_relay_template(context) else {
            return Ok(None);
        };
        relay_client.submit_dry_run(&template).await.map(Some)
    }

    fn insert_inner(&mut self, candidate: AssemblyCandidate) -> AssemblyDecision {
        if !candidate.simulation.approved {
            return self.reject(
                candidate.candidate_id,
                RejectionReason::SimulationNotApproved,
            );
        }

        let conflicts = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, existing)| candidates_conflict(existing, &candidate))
            .map(|(index, existing)| {
                (
                    index,
                    existing.candidate_id.clone(),
                    existing.priority_score,
                    existing.gas_used,
                )
            })
            .collect::<Vec<_>>();
        let replaced_candidate_ids = conflicts
            .iter()
            .map(|(_, candidate_id, _, _)| candidate_id.clone())
            .collect::<Vec<_>>();
        let conflicted_priority_score = conflicts.iter().fold(0_u64, |acc, (_, _, score, _)| {
            acc.saturating_add(*score as u64)
        });
        let conflicted_gas_used = conflicts.iter().fold(0_u64, |acc, (_, _, _, gas_used)| {
            acc.saturating_add(*gas_used)
        });
        // Equal-score ties preserve the incumbent candidate set. Replacements must be strictly
        // better to avoid churn between equivalent alternatives.
        if !replaced_candidate_ids.is_empty()
            && u64::from(candidate.priority_score) <= conflicted_priority_score
        {
            return self.reject(
                candidate.candidate_id,
                RejectionReason::ObjectiveNotImproved,
            );
        }

        let next_gas_used = self
            .live_objective()
            .total_gas_used
            .saturating_sub(conflicted_gas_used)
            .saturating_add(candidate.gas_used);
        if next_gas_used > self.config.block_gas_limit {
            return self.reject(
                candidate.candidate_id,
                RejectionReason::BlockGasLimitExceeded,
            );
        }

        let candidate_id = candidate.candidate_id.clone();
        for (index, _, _, _) in conflicts.into_iter().rev() {
            self.candidates.remove(index);
        }
        self.candidates.push(candidate);
        self.metrics.inserted_total = self.metrics.inserted_total.saturating_add(1);
        self.metrics.replaced_total = self
            .metrics
            .replaced_total
            .saturating_add(replaced_candidate_ids.len() as u64);
        self.refresh_live_metrics();
        AssemblyDecision::Inserted {
            candidate_id,
            replaced_candidate_ids,
        }
    }

    fn remove_candidate_inner(&mut self, candidate_id: &str) -> AssemblyRollbackDecision {
        let Some(index) = self
            .candidates
            .iter()
            .position(|candidate| candidate.candidate_id == candidate_id)
        else {
            return AssemblyRollbackDecision::NotFound {
                candidate_id: candidate_id.to_owned(),
            };
        };

        self.candidates.remove(index);
        self.metrics.rollback_total = self.metrics.rollback_total.saturating_add(1);
        self.refresh_live_metrics();
        AssemblyRollbackDecision::RolledBack {
            candidate_id: candidate_id.to_owned(),
        }
    }

    fn reject(&mut self, candidate_id: String, reason: RejectionReason) -> AssemblyDecision {
        self.metrics.rejected_total = self.metrics.rejected_total.saturating_add(1);
        match reason {
            RejectionReason::SimulationNotApproved => {
                self.metrics.rejected_simulation_not_approved_total = self
                    .metrics
                    .rejected_simulation_not_approved_total
                    .saturating_add(1);
            }
            RejectionReason::ObjectiveNotImproved => {
                self.metrics.rejected_objective_not_improved_total = self
                    .metrics
                    .rejected_objective_not_improved_total
                    .saturating_add(1);
            }
            RejectionReason::BlockGasLimitExceeded => {
                self.metrics.rejected_gas_limit_total =
                    self.metrics.rejected_gas_limit_total.saturating_add(1);
            }
        }
        AssemblyDecision::Rejected {
            candidate_id,
            reason: reason.as_str().to_owned(),
        }
    }

    fn compute_objective(&self) -> AssemblyObjective {
        self.candidates
            .iter()
            .fold(AssemblyObjective::default(), |mut objective, candidate| {
                objective.total_priority_score = objective
                    .total_priority_score
                    .saturating_add(candidate.priority_score as u64);
                objective.total_gas_used =
                    objective.total_gas_used.saturating_add(candidate.gas_used);
                objective.total_candidates = objective.total_candidates.saturating_add(1);
                objective
            })
    }

    fn refresh_live_metrics(&mut self) {
        let objective = self.compute_objective();
        self.metrics.active_candidate_total = self.candidates.len();
        self.metrics.total_priority_score = objective.total_priority_score;
        self.metrics.total_gas_used = objective.total_gas_used;
    }

    fn live_objective(&self) -> AssemblyObjective {
        AssemblyObjective {
            total_priority_score: self.metrics.total_priority_score,
            total_gas_used: self.metrics.total_gas_used,
            total_candidates: self.metrics.active_candidate_total,
        }
    }

    fn record_decision_latency(&mut self, elapsed: Duration) {
        let latency_ns = elapsed.as_nanos().clamp(1, u128::from(u64::MAX)) as u64;
        self.metrics.last_decision_latency_ns = latency_ns;
        self.metrics.max_decision_latency_ns = self.metrics.max_decision_latency_ns.max(latency_ns);
        self.metrics.total_decision_latency_ns = self
            .metrics
            .total_decision_latency_ns
            .saturating_add(latency_ns);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RejectionReason {
    SimulationNotApproved,
    ObjectiveNotImproved,
    BlockGasLimitExceeded,
}

impl RejectionReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::SimulationNotApproved => "simulation_not_approved",
            Self::ObjectiveNotImproved => "objective_not_improved",
            Self::BlockGasLimitExceeded => "block_gas_limit_exceeded",
        }
    }
}

fn candidates_conflict(left: &AssemblyCandidate, right: &AssemblyCandidate) -> bool {
    left.tx_hashes
        .iter()
        .any(|left_hash| right.tx_hashes.contains(left_hash))
}

// This is a deterministic internal identifier for relay dry-run templates, not an Ethereum
// execution-layer block hash.
fn compute_block_hash(context: &RelayBuildContext, candidates: &[AssemblyCandidate]) -> String {
    let mut digest = Sha256::new();
    digest.update(context.slot.to_le_bytes());
    digest.update(context.parent_hash.as_bytes());
    digest.update(context.builder_pubkey.as_bytes());
    for candidate in candidates {
        digest.update(candidate.candidate_id.as_bytes());
        digest.update(candidate.priority_score.to_le_bytes());
        digest.update(candidate.gas_used.to_le_bytes());
        for tx_hash in &candidate.tx_hashes {
            digest.update(tx_hash);
        }
    }
    let hash: [u8; 32] = digest.finalize().into();
    format!("0x{}", hex::encode(hash))
}

fn assembly_kind(strategy: StrategyKind, member_count: usize) -> AssemblyCandidateKind {
    if strategy == StrategyKind::BundleCandidate || member_count > 1 {
        AssemblyCandidateKind::Bundle
    } else {
        AssemblyCandidateKind::Transaction
    }
}
