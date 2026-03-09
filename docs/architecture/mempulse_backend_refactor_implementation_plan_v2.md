# Mempulse Backend Refactor v2

## Status

As of March 9, 2026, the backend refactor described by the v2 plan is implemented in the workspace.
The architecture is no longer in an intermediate milestone state.

## Final Architecture

### Scheduler authority

`crates/scheduler` is the single-writer authority for:

- mempool admission and replacement handling,
- sender queue readiness,
- candidate registration,
- simulation generation keys,
- stale simulation rejection,
- builder handoff records.

Heavy work still happens outside the scheduler, but state ownership does not.

### Runtime core role

`crates/runtime-core` is the live-path adapter around that authority:

- ingest and RPC fetch,
- remote-RPC simulation execution,
- builder engine interaction,
- live metrics and provider-backed state for `viz-api`.

`runtime-core` does not own candidate or simulation lifecycle truth. It routes those transitions through the scheduler and emits lifecycle events around them.

### Durable authority boundary

The durable sources of truth are:

- the event log,
- the persisted scheduler snapshot.

Flat storage tables are derived read models. `append_event` projects lifecycle data into those tables instead of treating direct upserts as competing authorities.

### Lifecycle contract

The lifecycle contract now includes:

- `CandidateQueued`,
- `SimDispatched`,
- `SimCompleted`,
- `AssemblyDecisionApplied`.

Replay reconstructs both:

- pending and sender-queue state,
- candidate, simulation, and assembly lifecycle state.

### Simulation backend

The active simulation path remains the planned Phase 1 shape:

- `revm` in-repo,
- remote RPC plus local cache for state reads,
- no in-repo execution client process.

Reth-backed state access remains optional future work behind the same scheduler-facing contract.

### Performance and CI

Critical-path perf coverage is now enforced for:

- tx pipeline baseline,
- scheduler pipeline,
- simulation roundtrip,
- storage snapshot write and rehydration.

CI compiles the bench targets, runs the perf regression script, and uploads the generated JSON artifacts.

## Completion Notes

The backend now matches the intended ownership model:

- scheduler owns runtime state transitions,
- runtime-core owns adapters and workers,
- storage is event-derived,
- replay is lifecycle-aware,
- perf gates cover the hot paths called out by the plan.

The remaining work after this point is incremental optimization or future backend enhancements, not architectural completion of the refactor itself.
