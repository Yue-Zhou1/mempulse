# Rust Best-Practices Refactor

**Date:** 2026-03-11
**Scope:** All 14 crates under `crates/`
**Reference:** Reth codebase conventions
**Approach:** Layered — three independently mergeable layers ordered by risk

---

## Context

Analysis of the crate workspace against Reth-style Rust conventions revealed three tiers of
improvement. Each tier is independent: Layer 1 carries zero logic risk, Layer 2 is structural
but preserves all public APIs, Layer 3 is the only tier that touches public struct fields.

**Current state at a glance:**
- `parking_lot` not in workspace — 9 poison-recovery boilerplate calls in `scheduler`
- `thiserror` is in workspace but only used by `ingest` — 7 other crates use `anyhow` in
  library positions
- `auto_impl` not in workspace — `VizDataProvider`, `EventStore`, and other traits require
  callers to hold `Arc<dyn Trait>` with no automatic blanket impls
- `#![forbid(unsafe_code)]` present in 6 of 14 crates; absent in 8
- `#[must_use]` and `#[inline]` absent across the entire workspace
- Stringly-typed `candidate_id: String` and `strategy: String` in `SchedulerCandidate`
- `normalize_config` silently clamps invalid values instead of returning an error

---

## Layer 1 — Mechanical (all 14 crates, zero logic changes)

### 1.1 `#![forbid(unsafe_code)]`

Add to the top of `lib.rs` (or `main.rs` for binaries) in the 8 crates that currently lack it:
`common`, `event-log`, `feature-engine`, `ingest`, `node-runtime`, `replay`, `storage`,
`viz-api`.

The 6 crates that already have it (`bench`, `builder`, `runtime-core`, `scheduler`, `searcher`,
`sim-engine`) need no change.

### 1.2 Replace `std::sync::RwLock` with `parking_lot::RwLock`

**Why:** `parking_lot::RwLock` never poisons. It eliminates the 9 instances of
`unwrap_or_else(|poison| poison.into_inner())` in `scheduler`, all of which are defensive
boilerplate that silently swallows panics in other threads. `parking_lot` also has lower
overhead than `std` on contended paths.

**Workspace change:** Add to `[workspace.dependencies]` in root `Cargo.toml`:
```toml
parking_lot = "0.12"
```

**Crate changes:**

| Crate | File | Current | After |
|---|---|---|---|
| `scheduler` | `src/lib.rs` | `use std::sync::{Arc, RwLock}` + 9× poison recovery | `use parking_lot::RwLock` |
| `runtime-core` | `src/lib.rs` | `use std::sync::{Arc, RwLock}` (7 `RwLock` fields) | `use parking_lot::RwLock` |
| `viz-api` | `src/lib.rs` | `use std::sync::{Arc, Mutex, RwLock}` (5 `RwLock` + 2 `Mutex` fields) | `use parking_lot::{Mutex, RwLock}` |

For `scheduler` specifically, every `state.read().unwrap_or_else(|poison| poison.into_inner())`
becomes `state.read()` — `parking_lot::RwLockReadGuard` is returned directly, no `Result` to
unwrap.

For `viz-api`, the two `Arc<Mutex<Option<tokio::task::AbortHandle>>>` fields
(`scheduler_snapshot_writer_abort`, `replay_runtime_metrics_abort`) become
`Arc<Mutex<Option<tokio::task::AbortHandle>>>` using `parking_lot::Mutex` — same type shape,
just the lock implementation changes.

Add `parking_lot = { workspace = true }` to `Cargo.toml` of `scheduler`, `runtime-core`,
`viz-api`.

### 1.3 `#[must_use]` on public APIs

Add `#[must_use]` to every public function whose return value is meaningful and must not be
silently dropped. Concrete list:

**`scheduler/src/lib.rs`**
- `SchedulerHandle::admit` — returns `SchedulerAdmissionOutcome`
- `SchedulerHandle::snapshot` — returns `Option<SchedulerSnapshot>`
- `SchedulerHandle::candidates` — returns `Result<Vec<SchedulerCandidate>, _>`
- All `send_command_with_reply` callers

**`storage/src/lib.rs`**
- `InMemoryStorage::append_event`
- `EventStore::events` (on the trait)
- `InMemoryStorage::snapshot`

**`common/src/lib.rs`**
- `evaluate_alerts` — returns `AlertDecisions`

**`replay/src/lib.rs`**
- `lifecycle_checkpoint_hash` — returns `Option<[u8; 32]>`
- `replay_diff_summary` — returns the diff struct

**`viz-api/src/lib.rs`**
- `VizDataProvider::latest_seq_id` (on the trait)
- `VizDataProvider::dashboard_snapshot` (on the trait)

### 1.4 `#[inline]` on thin accessors and constructors

Add `#[inline]` to methods whose body is a single expression, a field access, or a delegating
call. Concrete list:

**`common/src/lib.rs`**
- `SourceId::as_str`, `SourceId::Display` impl

**`scheduler/src/lib.rs`**
- `SchedulerHandle::new`
- All `impl SchedulerHandle` methods that call `send_command_with_reply`

**`storage/src/lib.rs`**
- `InMemoryStorage::new`, `InMemoryStorage::len`, `InMemoryStorage::is_empty`

**`viz-api/src/lib.rs`**
- All `AppState` field accessor methods

---

## Layer 2 — Structural (library crates, no public API shape change)

### 2.1 `thiserror` typed errors in library crates

**Principle (from Reth):** Library crates define their own error enums with `thiserror`.
Application crates (`viz-api`, `bench`, binaries) keep `anyhow`. This gives callers of library
crates the ability to match on specific error variants rather than downcasting `anyhow::Error`.

**New error types to define:**

#### `storage/src/lib.rs` → `StorageError`
```rust
#[derive(Clone, Debug, thiserror::Error)]
pub enum StorageError {
    #[error("WAL write failed: {0}")]
    WalWrite(Arc<dyn std::error::Error + Send + Sync>),
    #[error("ClickHouse batch failed: {0}")]
    ClickHouseBatch(Arc<dyn std::error::Error + Send + Sync>),
    #[error("Parquet export failed: {0}")]
    ParquetExport(Arc<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    Other(Arc<dyn std::error::Error + Send + Sync>),
}
```

Return type of `wal::write_batch`, `ClickHouseBatchSink::flush`, `ParquetExporter::export`
change from `anyhow::Result<_>` to `Result<_, StorageError>`.

#### `ingest/src/lib.rs` → `IngestError`
(Already uses `thiserror` in `Cargo.toml` — this is defining and using a proper enum)
```rust
#[derive(Clone, Debug, thiserror::Error)]
pub enum IngestError {
    #[error("RPC connection failed: {_0}")]
    RpcConnect(Arc<dyn std::error::Error + Send + Sync>),
    #[error("transaction fetch failed for {hash}: {source}")]
    TxFetch { hash: String, source: Arc<dyn std::error::Error + Send + Sync> },
    #[error(transparent)]
    Other(Arc<dyn std::error::Error + Send + Sync>),
}
```

#### `sim-engine/src/lib.rs` → `SimError`
```rust
#[derive(Clone, Debug, thiserror::Error)]
pub enum SimError {
    #[error("EVM execution reverted: {reason}")]
    Reverted { reason: String },
    #[error("state fetch failed: {0}")]
    StateFetch(Arc<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    Other(Arc<dyn std::error::Error + Send + Sync>),
}
```

#### `replay/src/lib.rs` → `ReplayError`
```rust
#[derive(Clone, Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("checkpoint mismatch at seq {seq}: expected {expected:?}, got {actual:?}")]
    CheckpointMismatch { seq: u64, expected: Box<[u8; 32]>, actual: Box<[u8; 32]> },
    #[error(transparent)]
    Other(Arc<dyn std::error::Error + Send + Sync>),
}
```

Note: `Box<[u8; 32]>` keeps the enum variant size small (pointer-sized) — mirrors the
`Box<LargeVariant>` pattern from Reth's `DatabaseError`.

**Crates that keep `anyhow`:** `builder`, `node-runtime`, `runtime-core`, `viz-api`, `bench`.
These are application-boundary crates where `anyhow`'s context chaining adds value and callers
do not need to match on variants.

**Cargo.toml changes:**
- Add `thiserror = { workspace = true }` to `storage`, `sim-engine`, `replay` `Cargo.toml`
- `ingest` already has it

### 2.2 `auto_impl` on shared traits

**Why:** `VizDataProvider` and other service traits are always held behind `Arc<dyn Trait>`.
Without `#[auto_impl(Arc)]`, any call site that has an `Arc<T: Trait>` cannot pass it to a
function expecting `&dyn Trait` without manual impl boilerplate. `auto_impl` generates the
blanket `impl Trait for Arc<T>`, `&T`, `Box<T>` automatically.

**Workspace change:** Add to `[workspace.dependencies]` in root `Cargo.toml`:
```toml
auto_impl = "1"
```

**Traits to annotate:**

| Trait | Crate | Annotation |
|---|---|---|
| `VizDataProvider` | `viz-api` | `#[auto_impl(&, Arc, Box)]` |
| `EventStore` | `storage` | `#[auto_impl(&, Arc, Box)]` |
| `ClickHouseBatchSink` | `storage` | `#[auto_impl(&, Arc, Box)]` |
| `StateProvider` | `sim-engine` | `#[auto_impl(&, Arc, Box)]` |
| `IngestClock` | `ingest` | `#[auto_impl(&, Arc)]` |
| `PendingTxProvider` | `ingest` | `#[auto_impl(&, Arc)]` |

Add `auto_impl = { workspace = true }` to `Cargo.toml` of `viz-api`, `storage`, `sim-engine`,
`ingest`.

---

## Layer 3 — Design-level (three targeted changes)

### 3.1 Newtype IDs for `SchedulerCandidate`

**Problem:** `SchedulerCandidate.candidate_id: String` and `.strategy: String` are
stringly-typed. A caller can pass them in the wrong order with no compile error.

**Change:** Add to `crates/common/src/lib.rs`:
```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CandidateId(pub String);

impl CandidateId {
    pub fn as_str(&self) -> &str { &self.0 }
}
impl fmt::Display for CandidateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}
impl From<String> for CandidateId {
    fn from(s: String) -> Self { Self(s) }
}
impl From<&str> for CandidateId {
    fn from(s: &str) -> Self { Self(s.to_owned()) }
}
impl std::ops::Deref for CandidateId {
    type Target = str;
    fn deref(&self) -> &str { &self.0 }
}
```

And `StrategyId` with identical shape.

**Update `scheduler/src/lib.rs`:**
```rust
pub struct SchedulerCandidate {
    pub candidate_id: CandidateId,   // was: String
    pub strategy: StrategyId,        // was: String
    // ... rest unchanged
}
```

All existing construction sites using `String` literals compile without change via
`From<&str>` / `From<String>`.

Also update `SimulationTaskSpec` in `sim-engine` and any other structs using raw `String` for
these semantic fields.

### 3.2 `OnceLock` for `ReplayRuntimeMetricsCache`

**Problem:** `ReplayRuntimeMetricsCache` in `viz-api/src/lib.rs` uses
`Arc<Mutex<Option<tokio::task::AbortHandle>>>` for the abort handles — a write-once-then-read
pattern wrapped in a general-purpose lock.

**Change:** Replace with `std::sync::OnceLock<tokio::task::AbortHandle>`. This matches the
Reth `SealedHeader` pattern (lazy, once-computed value; no lock overhead after first write).

```rust
// Before:
scheduler_snapshot_writer_abort: Arc<Mutex<Option<tokio::task::AbortHandle>>>,

// After:
scheduler_snapshot_writer_abort: Arc<OnceLock<tokio::task::AbortHandle>>,
```

On spawn: `abort_handle.set(handle).ok()` — no lock needed.
On abort: `if let Some(handle) = abort_handle.get() { handle.abort(); }` — no lock needed.

Remove the two `Mutex` imports in `viz-api/src/lib.rs` after this change.

### 3.3 Config validation returns `Result`

**Problem:** `SchedulerConfig::normalize_config` (scheduler/src/lib.rs) silently clamps
`handoff_queue_capacity = 0` to `1` and `max_pending_per_sender = 0` to `1`. The caller never
knows the config was invalid.

**Change:** Define:
```rust
#[derive(Clone, Debug, thiserror::Error)]
pub enum SchedulerConfigError {
    #[error("handoff_queue_capacity must be >= 1, got 0")]
    HandoffQueueCapacityZero,
    #[error("max_pending_per_sender must be >= 1, got 0")]
    MaxPendingPerSenderZero,
}
```

Change `normalize_config(config: SchedulerConfig) -> SchedulerConfig` to
`validate_config(config: SchedulerConfig) -> Result<SchedulerConfig, SchedulerConfigError>`.

All callers of `SchedulerConfig` construction already propagate `Result` (they call
`SchedulerHandle::new` which returns `anyhow::Result`), so the error threads through with `?`
at the call site.

---

## Dependency Summary

Changes to root `Cargo.toml` `[workspace.dependencies]`:
```toml
parking_lot = "0.12"   # new
auto_impl = "1"        # new
# thiserror = "2"      # already present
```

Crate-level `Cargo.toml` additions:

| Crate | New deps |
|---|---|
| `scheduler` | `parking_lot` |
| `runtime-core` | `parking_lot` |
| `viz-api` | `parking_lot`, `auto_impl` |
| `storage` | `thiserror`, `auto_impl` |
| `ingest` | `auto_impl` |
| `sim-engine` | `thiserror`, `auto_impl` |
| `replay` | `thiserror` |
| `common` | *(no new deps — newtype wrappers use only std+serde)* |

---

## Testing Strategy

Each layer must pass `cargo test --workspace` before the next begins.

- **Layer 1:** No logic change — `cargo clippy --workspace -D warnings` must pass and the 9
  poison-recovery patterns must be absent from `scheduler`
- **Layer 2:** All existing tests pass unchanged; additionally verify that `StorageError`,
  `IngestError`, `SimError`, `ReplayError` variants are tested with at least one unit test each
- **Layer 3:** `SchedulerConfig` validation tested with `capacity = 0` returning the correct
  error variant; `CandidateId`/`StrategyId` roundtrip through serde; `OnceLock` abort path
  tested by setting and calling `.abort()`

---

## Implementation Order

```
Layer 1
├── 1.1  forbid(unsafe_code) — all 8 missing crates
├── 1.2  parking_lot — scheduler, runtime-core, viz-api
├── 1.3  #[must_use] — all crates
└── 1.4  #[inline] — all crates

Layer 2
├── 2.1a  StorageError (storage)
├── 2.1b  IngestError (ingest)
├── 2.1c  SimError (sim-engine)
├── 2.1d  ReplayError (replay)
└── 2.2   auto_impl (viz-api, storage, sim-engine, ingest)

Layer 3
├── 3.1  CandidateId + StrategyId newtypes (common, scheduler, sim-engine)
├── 3.2  OnceLock abort handles (viz-api)
└── 3.3  SchedulerConfig validation Result (scheduler)
```
