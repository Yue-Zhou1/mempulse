# Transaction Handling Scale-Up Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Scale backend transaction ingest/processing from single-chain moderate flow to sustained high-volume multi-chain + L2 flow, while reducing per-transaction CPU/memory overhead and preserving deterministic behavior.

**Architecture:** Keep upstream node integration on Ethereum-compatible JSON-RPC (WS + HTTP), optimize hot path with zero-copy and bounded/sharded processing, remove O(N) scans in serve paths, and add chain-aware sharding plus strict backpressure and performance gates. Evaluate gRPC only as an internal service boundary after hot-path optimizations land.

**Tech Stack:** Rust (`tokio`, `axum`, `tokio-tungstenite`, `serde`), existing `storage` + `viz-api` crates, optional `tonic` (internal gRPC phase), existing WAL/ClickHouse pipeline.

## Scope and Success Criteria

### In Scope
- Backend ingest and backend API serving path only.
- Transaction fetch/decode/feature/ranking/event-write performance.
- Multi-chain (EVM mainnets + L2s) ingest architecture.
- Zero-copy/low-copy memory optimization on critical path.
- Reliability under overload (bounded queues + drop observability).

### Out of Scope (this plan)
- Frontend rendering changes.
- Non-EVM chain protocols.
- Replacing upstream Ethereum JSON-RPC with a non-standard node protocol.

### Target SLOs (release gate)
- Sustained ingest: `>= 5x` current baseline tx/sec on same hardware.
- P99 ingest-to-visible latency: `< 700 ms` under target load.
- No unbounded memory growth in backend over 30 min stress run.
- Queue overflow/drop ratio: `< 0.5%` at target steady-state load.
- Recovery: process restart + WAL recovery returns to healthy state with no panic/data corruption.

---

### Task 1: Baseline and Perf Harness (must land first)

**Files:**
- Create: `crates/viz-api/tests/tx_pipeline_perf_baseline.rs`
- Create: `scripts/perf_tx_pipeline_baseline.sh`
- Modify: `crates/viz-api/Cargo.toml`
- Modify: `README.md`

**Step 1: Write the failing test**

Add a test harness that:
- Seeds deterministic synthetic tx events.
- Measures snapshot endpoint latency and stream catch-up time.
- Asserts baseline metrics are emitted to stdout/JSON artifact.

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api --test tx_pipeline_perf_baseline -- --nocapture`  
Expected: FAIL because harness/fixtures are not wired yet.

**Step 3: Write minimal implementation**

- Add test fixture helpers and deterministic seeding.
- Add script to run baseline locally and store artifact in `artifacts/perf/`.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p viz-api --test tx_pipeline_perf_baseline -- --nocapture`
- `bash scripts/perf_tx_pipeline_baseline.sh`

Expected: PASS; artifact generated.

**Step 5: Commit**

```bash
git add crates/viz-api/tests/tx_pipeline_perf_baseline.rs scripts/perf_tx_pipeline_baseline.sh crates/viz-api/Cargo.toml README.md
git commit -m "test: add backend tx pipeline baseline harness"
```

---

### Task 2: Multi-Chain Ingest Topology and Config

**Files:**
- Modify: `crates/viz-api/src/live_rpc.rs`
- Modify: `crates/viz-api/src/lib.rs`
- Modify: `crates/storage/src/lib.rs`
- Create: `crates/viz-api/tests/multi_chain_ingest_config.rs`
- Modify: `.env.example` (or backend env docs file)

**Step 1: Write the failing test**

Add tests for:
- Parsing multiple chain endpoints from env.
- Spawning one ingest worker per configured chain.
- Correct chain tag propagation to storage-facing records.

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api multi_chain_ingest_config`  
Expected: FAIL (no multi-chain config parser/worker wiring yet).

**Step 3: Write minimal implementation**

- Introduce chain-scoped config model (chain key + WS/HTTP endpoints + source id).
- Spawn per-chain ingest tasks with isolated dedup sets and queue metrics.
- Ensure chain metadata is stored with tx/full/feature/opportunity records (new fields if missing).

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p viz-api multi_chain_ingest_config`
- `cargo test -p storage --lib`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/live_rpc.rs crates/viz-api/src/lib.rs crates/storage/src/lib.rs crates/viz-api/tests/multi_chain_ingest_config.rs .env.example
git commit -m "feat: add chain-sharded ingest config and worker topology"
```

---

### Task 3: Zero-Copy WS Pending Hash Parse

**Files:**
- Modify: `crates/viz-api/src/live_rpc.rs`
- Create: `crates/viz-api/tests/live_rpc_pending_parse.rs`

**Step 1: Write the failing test**

Add tests for `parse_pending_hash` using borrowed JSON models:
- valid subscription handshake
- valid hash payload
- non-matching subscription id
- malformed payload

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api live_rpc_pending_parse`  
Expected: FAIL before refactor.

**Step 3: Write minimal implementation**

- Replace `serde_json::Value` parsing for WS path with typed borrowed structs (`&str`/`Cow<'_, str>`).
- Return borrowed hash where possible; allocate only when crossing async/task boundaries.

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api live_rpc_pending_parse`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/live_rpc.rs crates/viz-api/tests/live_rpc_pending_parse.rs
git commit -m "perf: use borrowed json parsing for pending-hash websocket path"
```

---

### Task 4: Zero-Copy HTTP RPC Decode (`eth_getTransactionByHash`)

**Files:**
- Modify: `crates/viz-api/src/live_rpc.rs`
- Create: `crates/viz-api/tests/live_rpc_fetch_decode.rs`

**Step 1: Write the failing test**

Add tests asserting:
- decode works from raw bytes directly
- no intermediate `Value` clone required
- behavior preserved on error/null result

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api live_rpc_fetch_decode`  
Expected: FAIL before refactor.

**Step 3: Write minimal implementation**

- Replace:
  - `response.json::<Value>()` + `serde_json::from_value(...)`
- With:
  - `response.bytes()` + `serde_json::from_slice(...)` into typed envelope with borrowed fields.
- Keep existing error handling semantics.

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api live_rpc_fetch_decode`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/live_rpc.rs crates/viz-api/tests/live_rpc_fetch_decode.rs
git commit -m "perf: remove intermediate value clone in rpc tx decode path"
```

---

### Task 5: Remove Calldata Copy in Search/Feature Path

**Files:**
- Modify: `crates/viz-api/src/live_rpc.rs`
- Modify: `crates/searcher/src/lib.rs` (or corresponding searcher input crate)
- Create: `crates/viz-api/tests/opportunity_zero_copy.rs`

**Step 1: Write the failing test**

Add a test that enforces searcher input accepts borrowed calldata (`&[u8]`) and preserves output.

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api opportunity_zero_copy`  
Expected: FAIL due to current `calldata.to_vec()` behavior.

**Step 3: Write minimal implementation**

- Change searcher input types to borrow calldata.
- Replace `calldata.to_vec()` with borrow/reference.
- Keep ownership only at persistence boundaries where needed.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p viz-api opportunity_zero_copy`
- `cargo test -p searcher`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/live_rpc.rs crates/searcher/src/lib.rs crates/viz-api/tests/opportunity_zero_copy.rs
git commit -m "perf: eliminate calldata clone in feature and opportunity ranking path"
```

---

### Task 6: Batch RPC Fetch + Bounded Concurrency per Chain

**Files:**
- Modify: `crates/viz-api/src/live_rpc.rs`
- Create: `crates/viz-api/tests/live_rpc_batch_fetch.rs`
- Modify: backend env docs for new knobs

**Step 1: Write the failing test**

Add tests for:
- request batching size limits
- bounded in-flight requests per chain
- retry/backoff and fallback endpoint behavior

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api live_rpc_batch_fetch`  
Expected: FAIL before batching implementation.

**Step 3: Write minimal implementation**

- Add chain-scoped fetch queue.
- Coalesce pending hashes into JSON-RPC batch calls.
- Apply semaphore-based in-flight cap per chain.
- Emit metrics for batch size, RPC latency, error rate.

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api live_rpc_batch_fetch`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/live_rpc.rs crates/viz-api/tests/live_rpc_batch_fetch.rs
git commit -m "feat: add batched tx detail rpc fetch with bounded per-chain concurrency"
```

---

### Task 7: Replace O(N) Event Scan with Cursor/Index Access

**Files:**
- Modify: `crates/storage/src/lib.rs`
- Modify: `crates/viz-api/src/lib.rs`
- Create: `crates/storage/tests/scan_events_cursor.rs` (or lib tests if preferred)

**Step 1: Write the failing test**

Add tests that:
- validate deterministic ordering
- validate cursor scan correctness
- assert scan complexity remains stable with large event buffers (sanity bound)

**Step 2: Run test to verify it fails**

Run: `cargo test -p storage scan_events_cursor`  
Expected: FAIL before cursor/index implementation.

**Step 3: Write minimal implementation**

- Stop using `list_events()` clone + sort for each scan.
- Store events in sequence order with index/cursor lookup.
- Implement `scan_events(from_seq_id, limit)` using cursor/binary-search semantics.
- Keep deterministic semantics unchanged.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p storage scan_events_cursor`
- `cargo test -p viz-api --test api_auth_and_limits`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/storage/src/lib.rs crates/viz-api/src/lib.rs crates/storage/tests/scan_events_cursor.rs
git commit -m "perf: implement indexed event scan and remove full-buffer cloning"
```

---

### Task 8: Snapshot Read Path Caching (Incremental Aggregates)

**Files:**
- Modify: `crates/viz-api/src/lib.rs`
- Modify: `crates/storage/src/lib.rs`
- Create: `crates/viz-api/tests/dashboard_snapshot_perf.rs`

**Step 1: Write the failing test**

Add tests asserting:
- repeated snapshot calls do not recompute full aggregates each time
- snapshot output remains deterministic and unchanged functionally

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api dashboard_snapshot_perf`  
Expected: FAIL before cache/incremental state.

**Step 3: Write minimal implementation**

- Maintain incremental read models for:
  - replay points (or cached latest windows)
  - feature summary counts
  - recent feature details
  - opportunities window
- Invalidate/update cache only on new events.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p viz-api dashboard_snapshot_perf`
- `cargo test -p viz-api`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/lib.rs crates/storage/src/lib.rs crates/viz-api/tests/dashboard_snapshot_perf.rs
git commit -m "perf: add incremental dashboard snapshot caches for high-volume reads"
```

---

### Task 9: Backpressure Hardening and Drop Telemetry

**Files:**
- Modify: `crates/storage/src/lib.rs`
- Modify: `crates/viz-api/src/live_rpc.rs`
- Modify: `crates/viz-api/src/lib.rs`
- Create: `crates/viz-api/tests/queue_drop_policy.rs`

**Step 1: Write the failing test**

Add tests to verify:
- bounded queue behavior under overload
- explicit drop reason classification
- drop metrics surfaced in API metrics endpoint

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api queue_drop_policy`  
Expected: FAIL before new telemetry and policies.

**Step 3: Write minimal implementation**

- Use `try_enqueue` in overload-sensitive paths where blocking is harmful.
- Define drop reason enum and counters.
- Emit structured logs and metrics per chain/source/queue.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p viz-api queue_drop_policy`
- `cargo test -p storage --lib`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/storage/src/lib.rs crates/viz-api/src/live_rpc.rs crates/viz-api/src/lib.rs crates/viz-api/tests/queue_drop_policy.rs
git commit -m "feat: enforce overload drop policy with per-chain queue telemetry"
```

---

### Task 10: Chain-Aware API Query Surface

**Files:**
- Modify: `crates/viz-api/src/lib.rs`
- Modify: `crates/storage/src/lib.rs`
- Create: `crates/viz-api/tests/api_chain_filters.rs`
- Modify: API docs/README for route query params

**Step 1: Write the failing test**

Add tests for `chain_id` filters on:
- `/dashboard/snapshot`
- `/transactions`, `/transactions/all`
- `/features/recent`
- `/opps/recent`

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api api_chain_filters`  
Expected: FAIL before chain-aware filtering.

**Step 3: Write minimal implementation**

- Add optional `chain_id` query fields.
- Filter at provider/storage read path without additional full clones.
- Keep existing defaults backward-compatible.

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api api_chain_filters`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/lib.rs crates/storage/src/lib.rs crates/viz-api/tests/api_chain_filters.rs README.md
git commit -m "feat: add chain-aware filtering to transaction and dashboard APIs"
```

---

### Task 11: WAL/Flush Path Throughput Tuning

**Files:**
- Modify: `crates/storage/src/lib.rs`
- Modify: `crates/storage/src/wal.rs`
- Create: `crates/storage/tests/wal_flush_throughput.rs`

**Step 1: Write the failing test**

Add throughput and correctness tests for:
- larger flush batches
- configurable flush intervals
- WAL append + clear behavior under high event rates

**Step 2: Run test to verify it fails**

Run: `cargo test -p storage wal_flush_throughput`  
Expected: FAIL before tuning/wire-up.

**Step 3: Write minimal implementation**

- Add explicit tuning knobs (batch size/interval defaults for high volume).
- Ensure WAL segment handling remains safe and deterministic.
- Keep shutdown flush guarantees.

**Step 4: Run test to verify it passes**

Run: `cargo test -p storage wal_flush_throughput`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/storage/src/lib.rs crates/storage/src/wal.rs crates/storage/tests/wal_flush_throughput.rs
git commit -m "perf: tune writer and wal flush path for high-throughput ingest"
```

---

### Task 12: Internal gRPC Evaluation and Optional Integration Spike

**Files:**
- Create: `docs/plans/2026-02-25-internal-grpc-spike-notes.md`
- Optional Create: `crates/viz-api/src/grpc.rs`
- Optional Modify: `crates/viz-api/Cargo.toml`
- Optional Create: `proto/viz_ingest.proto`

**Step 1: Write the failing test (if implementing spike code)**

Add contract test for internal gRPC stream equivalence vs existing in-process event bus.

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api grpc_spike`  
Expected: FAIL before spike implementation.

**Step 3: Write minimal implementation**

- Document and/or implement limited spike:
  - gRPC ingestion service interface
  - protobuf payload for normalized tx envelope
  - compare CPU/memory/latency against in-process path

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api grpc_spike` (if code added)  
Expected: PASS.

**Step 5: Commit**

```bash
git add docs/plans/2026-02-25-internal-grpc-spike-notes.md crates/viz-api/src/grpc.rs crates/viz-api/Cargo.toml proto/viz_ingest.proto
git commit -m "spike: evaluate internal grpc transport for service boundary"
```

---

### Task 13: CI Performance Gates and Regression Protection

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `scripts/perf_regression_check.sh`
- Create: `artifacts/perf/.gitkeep`

**Step 1: Write the failing check**

Add CI job that compares current performance artifact against baseline budget.

**Step 2: Run check to verify it fails locally**

Run: `bash scripts/perf_regression_check.sh`  
Expected: FAIL until baseline and thresholds are configured.

**Step 3: Write minimal implementation**

- Define thresholds for:
  - snapshot latency
  - ingest throughput
  - drop ratio
- Fail CI when regression exceeds budget.

**Step 4: Run check to verify it passes**

Run: `bash scripts/perf_regression_check.sh`  
Expected: PASS with current tuned baseline.

**Step 5: Commit**

```bash
git add .github/workflows/ci.yml scripts/perf_regression_check.sh artifacts/perf/.gitkeep
git commit -m "ci: add backend perf regression gates for tx handling pipeline"
```

---

## Rollout Plan

### Phase A (Week 1): Mandatory throughput unlock
- Task 1, 3, 4, 5, 7.
- Exit criteria: no behavior regressions, >=2x throughput improvement on baseline hardware.

### Phase B (Week 2): Multi-chain readiness
- Task 2, 6, 9, 10.
- Exit criteria: stable multi-chain ingest with bounded drops and chain-aware querying.

### Phase C (Week 3): Durability and production hardening
- Task 8, 11, 13.
- Exit criteria: CI perf gates enabled, restart recovery validated, no unbounded growth.

### Phase D (Optional): Internal transport evolution
- Task 12.
- Exit criteria: objective data supports keeping current in-process path or moving selected boundaries to gRPC.

---

## Risk Register and Mitigation

- **Risk:** Over-optimization introduces semantic regressions in ordering.
  - **Mitigation:** Determinism tests for event ordering and replay digests on every phase.
- **Risk:** Batching increases tail latency under low traffic.
  - **Mitigation:** flush-on-timeout + small max wait windows.
- **Risk:** Multi-chain fanout causes lock contention.
  - **Mitigation:** per-chain sharding and reduced global lock critical sections.
- **Risk:** Optional gRPC spike adds complexity without ROI.
  - **Mitigation:** keep spike isolated and evidence-driven before adoption.

---

## Definition of Done

- All mandatory tasks complete with passing tests.
- Performance baselines stored and enforced in CI.
- Multi-chain ingest and query paths validated.
- Zero-copy/low-copy changes merged for WS parse, HTTP decode, and calldata handling.
- O(N) scan hotspots removed from critical paths.
- Operational docs updated with tuning knobs and rollout guidance.

