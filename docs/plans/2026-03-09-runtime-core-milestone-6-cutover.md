# Runtime Core Milestone 6 Cutover

## Status

Milestone 6 is complete as of March 9, 2026.

The original cutover goal was to remove runtime-owned mutable state from `viz-api` and move it behind a dedicated runtime seam. That seam now exists in `crates/runtime-core`, and the final live path is narrower and cleaner than the intermediate milestone wording assumed.

## Final State

### `viz-api`

`viz-api` is a provider-backed control plane. It reads snapshots, metrics, and replay data from runtime/storage providers instead of owning live mutable state.

### `runtime-core`

`runtime-core` owns:

- live RPC ingest and fetch orchestration,
- simulation worker execution,
- builder engine interaction,
- runtime-local metrics and status views.

It does not own candidate lifecycle authority.

### `scheduler`

`scheduler` owns:

- candidate registration,
- simulation task generation,
- stale result invalidation,
- builder handoff decisions,
- nonce-aware mempool readiness.

`runtime-core::live_rpc` now behaves as an adapter that talks to the scheduler instead of as an authority that keeps its own competing candidate state.

### Persistence and replay

The event log and persisted scheduler snapshot are the durable truth for restart and replay.

Storage projections are derived from appended lifecycle events, including:

- opportunity rows from `CandidateQueued`,
- builder lifecycle rows from `AssemblyDecisionApplied`.

Replay reconstructs candidate simulation and assembly status in addition to pending mempool state.

## Verification Highlights

The milestone closeout depends on these behaviors, which are now covered by tests:

- runtime-core registers candidates through the scheduler,
- stale simulation results are discarded after replacement or head advance,
- lifecycle events are emitted for queue, dispatch, simulation completion, and assembly decision,
- storage and replay reconstruct the same lifecycle from durable events.

## Deferred Work

The cutover intentionally does not add a new simulation backend. Remote RPC plus local cache remains the active simulation mode, with any future Reth-backed state access kept behind the same scheduler-facing interface.
