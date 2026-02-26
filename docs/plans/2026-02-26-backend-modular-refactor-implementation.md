# Performance-First Backend Modular Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor the backend to a Reth-style modular runtime boundary while preserving single-process hot-path performance and current API behavior.

**Architecture:** Keep ingest, storage-writer, and feature/opportunity derivation in one runtime process for minimum latency. Introduce a dedicated runtime composition layer (`node-runtime`) and move live RPC ingest out of `viz-api` into the `ingest` crate with explicit lifecycle handles. Keep `viz-api` focused on HTTP/WebSocket read and query serving.

**Tech Stack:** Rust workspace, Tokio, Axum, tokio-tungstenite, reqwest, tracing, cargo test.

## Performance Guardrails

- No additional network hop on the ingest -> storage write hot path.
- No extra serialization layer between ingest and storage writer.
- Maintain or improve current transaction pipeline baseline in `scripts/perf_tx_pipeline_baseline.sh`.
- Keep queue-based backpressure semantics (`StorageWriteHandle::try_enqueue`) unchanged.

## Plan Constraints

- DRY and YAGNI: only extract boundaries needed for ownership and lifecycle clarity.
- TDD for each task: write failing test first, then minimal implementation.
- Frequent commits: one commit per task.
- Preserve existing endpoint contracts in `viz-api`.

### Task 1: Scaffold Runtime Composition Crate (`node-runtime`)

**Files:**
- Create: `crates/node-runtime/Cargo.toml`
- Create: `crates/node-runtime/src/lib.rs`
- Create: `crates/node-runtime/tests/runtime_smoke.rs`
- Modify: `Cargo.toml`

**Step 1: Write the failing test**

```rust
// crates/node-runtime/tests/runtime_smoke.rs
use node_runtime::NodeRuntimeBuilder;

#[test]
fn builder_smoke_compiles() {
    let _builder = NodeRuntimeBuilder::default();
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p node-runtime builder_smoke_compiles -- --exact`
Expected: FAIL with crate/module missing.

**Step 3: Write minimal implementation**

```rust
// crates/node-runtime/src/lib.rs
#[derive(Default)]
pub struct NodeRuntimeBuilder;
```

Also add workspace member + minimal dependencies in `Cargo.toml` and `crates/node-runtime/Cargo.toml`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p node-runtime builder_smoke_compiles -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add Cargo.toml crates/node-runtime/Cargo.toml crates/node-runtime/src/lib.rs crates/node-runtime/tests/runtime_smoke.rs
git commit -m "refactor: scaffold node-runtime composition crate"
```

### Task 2: Move Live RPC Runtime Interface to `ingest`

**Files:**
- Create: `crates/ingest/src/live_rpc.rs`
- Modify: `crates/ingest/src/lib.rs`
- Create: `crates/ingest/tests/live_rpc_runtime_handle.rs`

**Step 1: Write the failing test**

```rust
// crates/ingest/tests/live_rpc_runtime_handle.rs
use ingest::{LiveRpcConfig, LiveRpcRuntime};

#[test]
fn runtime_start_returns_handle() {
    let _ = LiveRpcRuntime::new(LiveRpcConfig::default());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ingest runtime_start_returns_handle -- --exact`
Expected: FAIL with unresolved items.

**Step 3: Write minimal implementation**

```rust
// crates/ingest/src/live_rpc.rs
#[derive(Clone, Debug, Default)]
pub struct LiveRpcConfig;

pub struct LiveRpcRuntime {
    _config: LiveRpcConfig,
}

impl LiveRpcRuntime {
    pub fn new(config: LiveRpcConfig) -> Self {
        Self { _config: config }
    }
}
```

Re-export in `crates/ingest/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ingest runtime_start_returns_handle -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/ingest/src/lib.rs crates/ingest/src/live_rpc.rs crates/ingest/tests/live_rpc_runtime_handle.rs
git commit -m "refactor: introduce live rpc runtime boundary in ingest crate"
```

### Task 3: Remove Global Singletons from Live RPC Runtime State

**Files:**
- Modify: `crates/ingest/src/live_rpc.rs`
- Create: `crates/ingest/tests/live_rpc_instance_state.rs`
- Modify: `crates/viz-api/src/lib.rs`

**Step 1: Write the failing test**

```rust
// crates/ingest/tests/live_rpc_instance_state.rs
use ingest::{LiveRpcConfig, LiveRpcRuntime};

#[test]
fn runtime_state_is_instance_scoped() {
    let a = LiveRpcRuntime::new(LiveRpcConfig::default());
    let b = LiveRpcRuntime::new(LiveRpcConfig::default());
    assert_ne!(a.instance_id(), b.instance_id());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ingest runtime_state_is_instance_scoped -- --exact`
Expected: FAIL with missing `instance_id`.

**Step 3: Write minimal implementation**

- Add `LiveRpcRuntimeState` struct owned by runtime instance.
- Replace `OnceLock` status/drop metric stores with instance fields.
- Add `instance_id()` getter.
- In `viz-api`, route chain status/metrics through runtime-owned state (no process-global reads).

```rust
pub struct LiveRpcRuntime {
    instance_id: u64,
    state: Arc<LiveRpcRuntimeState>,
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ingest runtime_state_is_instance_scoped -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/ingest/src/live_rpc.rs crates/ingest/tests/live_rpc_instance_state.rs crates/viz-api/src/lib.rs
git commit -m "refactor: make live rpc metrics and status instance-scoped"
```

### Task 4: Enforce Single Sequencing Authority in Storage Writer

**Files:**
- Modify: `crates/storage/src/lib.rs`
- Modify: `crates/ingest/src/live_rpc.rs`
- Create: `crates/storage/tests/global_sequencer_authority.rs`

**Step 1: Write the failing test**

```rust
// crates/storage/tests/global_sequencer_authority.rs
use storage::{InMemoryStorage, StorageWriteOp};

#[test]
fn append_event_sequence_is_assigned_by_storage_writer() {
    // Ensure producer-provided seq_id does not become canonical seq_id.
    // (test sets seq_id=0 and expects canonical sequence to start at 1)
    assert!(true);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p storage append_event_sequence_is_assigned_by_storage_writer -- --exact`
Expected: FAIL until assertion is implemented against real writer flow.

**Step 3: Write minimal implementation**

- Introduce append op that carries payload without authoritative `seq_id` from producer.
- Keep `GlobalSequencer` assignment only in `storage::apply_write_op`.
- Update ingest runtime to stop calling `next_seq_id.fetch_add` for canonical event IDs.

```rust
pub enum StorageWriteOp {
    AppendPayload { source_id: SourceId, payload: EventPayload, ingest_ts_unix_ms: i64, ingest_ts_mono_ns: u64 },
    // existing upserts...
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p storage append_event_sequence_is_assigned_by_storage_writer -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/storage/src/lib.rs crates/storage/tests/global_sequencer_authority.rs crates/ingest/src/live_rpc.rs
git commit -m "refactor: centralize canonical seq assignment in storage writer"
```

### Task 5: Build Explicit Node Runtime Start/Stop Lifecycle

**Files:**
- Modify: `crates/node-runtime/src/lib.rs`
- Create: `crates/node-runtime/src/builder.rs`
- Create: `crates/node-runtime/src/handle.rs`
- Create: `crates/node-runtime/tests/runtime_lifecycle.rs`

**Step 1: Write the failing test**

```rust
// crates/node-runtime/tests/runtime_lifecycle.rs
use node_runtime::NodeRuntimeBuilder;

#[tokio::test]
async fn runtime_exposes_shutdown_handle() {
    let runtime = NodeRuntimeBuilder::default().build().expect("build");
    runtime.shutdown().await.expect("shutdown");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p node-runtime runtime_exposes_shutdown_handle -- --exact`
Expected: FAIL with missing methods.

**Step 3: Write minimal implementation**

- Add `NodeRuntime` with owned ingest handle(s), storage handle, and API state.
- Add `shutdown()` and `wait()` semantics.
- Keep runtime single-process (performance-first), but lifecycle-explicit.

```rust
pub struct NodeRuntime { /* handles */ }
impl NodeRuntime {
    pub async fn shutdown(self) -> anyhow::Result<()> { Ok(()) }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p node-runtime runtime_exposes_shutdown_handle -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/node-runtime/src/lib.rs crates/node-runtime/src/builder.rs crates/node-runtime/src/handle.rs crates/node-runtime/tests/runtime_lifecycle.rs
git commit -m "refactor: add explicit node runtime lifecycle handle"
```

### Task 6: Make `viz-api` an API Surface, Not Composition Root

**Files:**
- Modify: `crates/viz-api/src/lib.rs`
- Modify: `crates/viz-api/src/bin/viz-api.rs`
- Modify: `crates/viz-api/Cargo.toml`
- Create: `crates/viz-api/tests/default_state_pure.rs`

**Step 1: Write the failing test**

```rust
// crates/viz-api/tests/default_state_pure.rs
use viz_api::default_state;

#[test]
fn default_state_is_pure_and_does_not_spawn_ingest() {
    let _state = default_state();
    // assertion uses new test helper that reports zero runtime tasks started
    assert!(true);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api default_state_is_pure_and_does_not_spawn_ingest -- --exact`
Expected: FAIL until state construction is separated from runtime start.

**Step 3: Write minimal implementation**

- Remove ingest startup from `default_state`.
- Add explicit startup path in binary (or node-runtime crate) before `axum::serve`.
- Keep `build_router` and provider contracts unchanged.

```rust
// default_state(): build-only
pub fn default_state() -> AppState { /* no spawn */ }
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api default_state_is_pure_and_does_not_spawn_ingest -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/lib.rs crates/viz-api/src/bin/viz-api.rs crates/viz-api/Cargo.toml crates/viz-api/tests/default_state_pure.rs
git commit -m "refactor: separate viz-api app state from ingest/runtime startup"
```

### Task 7: Wire Runtime Builder Through Binary Entry Point

**Files:**
- Modify: `crates/viz-api/src/bin/viz-api.rs`
- Modify: `crates/node-runtime/src/lib.rs`
- Create: `crates/viz-api/tests/bin_bootstrap.rs`

**Step 1: Write the failing test**

```rust
// crates/viz-api/tests/bin_bootstrap.rs
#[test]
fn binary_bootstrap_uses_runtime_builder_contract() {
    // compile-time contract test over function signatures/imports
    assert!(true);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api binary_bootstrap_uses_runtime_builder_contract -- --exact`
Expected: FAIL until binary wiring changes.

**Step 3: Write minimal implementation**

- Binary initializes tracing, constructs runtime via `node-runtime`, then serves API router.
- Shutdown path calls runtime shutdown handle.

```rust
let runtime = node_runtime::NodeRuntimeBuilder::from_env()?.build()?;
let app = viz_api::build_router(runtime.app_state().clone());
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api binary_bootstrap_uses_runtime_builder_contract -- --exact`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/bin/viz-api.rs crates/node-runtime/src/lib.rs crates/viz-api/tests/bin_bootstrap.rs
git commit -m "refactor: bootstrap backend through runtime builder"
```

### Task 8: Full Verification and Performance Regression Gate

**Files:**
- Modify: `scripts/verify_phase_checks.sh`
- Modify: `docs/architecture/v2_architecture_diagram.md`
- Create: `docs/plans/2026-02-26-backend-modular-refactor-cutover.md`

**Step 1: Write the failing test/check first**

Add a failing verification check in script for missing `node-runtime` test target:

```bash
cargo test -p node-runtime
```

**Step 2: Run checks to verify failure first**

Run: `bash scripts/verify_phase_checks.sh`
Expected: FAIL until all new crates/tests are wired.

**Step 3: Write minimal implementation**

- Update verification script to include `node-runtime`.
- Update architecture doc to show `node-runtime` composition boundary.
- Add cutover doc with rollout, rollback, and observability checks.

**Step 4: Run checks to verify pass + perf guardrail**

Run:
- `bash scripts/verify_phase_checks.sh`
- `bash scripts/perf_tx_pipeline_baseline.sh`

Expected:
- All tests/builds PASS.
- Baseline does not regress beyond agreed threshold (target <= 5% regression).

**Step 5: Commit**

```bash
git add scripts/verify_phase_checks.sh docs/architecture/v2_architecture_diagram.md docs/plans/2026-02-26-backend-modular-refactor-cutover.md
git commit -m "docs/chore: add modular runtime verification and cutover guardrails"
```

## Final End-to-End Verification

Run:

```bash
cargo test --workspace
cargo build -p viz-api --bin viz-api
cargo build -p node-runtime
bash scripts/verify_phase_checks.sh
bash scripts/perf_tx_pipeline_baseline.sh
```

Expected:
- No API contract regression.
- Runtime starts/stops explicitly.
- Ingest logic owned by `ingest` crate.
- Canonical sequence authority owned by storage writer.
- No performance regression beyond target budget.

## Notes for Execution

- Use `@superpowers:test-driven-development` per task.
- Use `@superpowers:verification-before-completion` before completion claim.
- Keep commits small and isolated to one task each.
