#![forbid(unsafe_code)]

//! Block-assembly primitives and relay dry-run utilities.

mod assembly;
mod relay_client;

pub use assembly::{
    AssemblyCandidate, AssemblyCandidateBuildError, AssemblyCandidateKind, AssemblyConfig,
    AssemblyDecision, AssemblyEngine, AssemblyMetrics, AssemblyObjective, AssemblyRollbackDecision,
    AssemblySnapshot, RelayBuildContext, SimulationApproval,
};
pub use relay_client::{
    BlockTemplate, RelayAttemptTrace, RelayClient, RelayClientConfig, RelayDryRunResult,
    RelayDryRunStatus,
};
