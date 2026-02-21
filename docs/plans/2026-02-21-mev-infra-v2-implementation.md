# MEV Infrastructure V2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Upgrade the current prototype into an interview-grade, end-to-end Ethereum mempool + MEV research and block-building pipeline aligned with Sigma Prime role expectations.

**Architecture:** Keep the current event-driven Rust pipeline as the backbone, then add a production p2p ingest lane, deterministic execution/simulation, opportunity engine, PBS relay integration, and performance/ops hardening. Preserve replay determinism and make every major claim measurable via benchmark and SLO artifacts.

**Tech Stack:** Rust, Tokio, Axum, Reth networking (devp2p), revm, Prometheus, ClickHouse, Grafana OSS, criterion/iai-callgrind/flamegraph, Docker Compose.

## JD Alignment Matrix

| JD Requirement | Current State | V2 Deliverable |
|---|---|---|
| High-performance Rust, latency optimization | Partial | Benchmarked hot path with allocator/runtime tuning and p50/p99 stage budgets |
| Deep EVM execution mechanics | Limited | Deterministic revm simulation lane and gas/state access analytics |
| MEV ecosystem end-to-end (PBS, relays) | Missing | Relay adapter, bid strategy skeleton, block template lifecycle |
| MEV searching experience | Missing | Opportunity engine + backtest report on replayed traces |
| Transaction supply chain understanding | Partial | Real devp2p ingest, propagation/coverage analysis, private flow stubs |
| Production systems rigor | Partial | SLO dashboards, alerts, chaos tests, runbooks, reproducible demo pack |

## Acceptance Targets (Interview Readiness)

1. Process and feature live mainnet mempool feed continuously for 60+ minutes without OOM or queue collapse.
2. Show measurable ingest latency and coverage delta between RPC and devp2p modes.
3. Replay deterministic parity >99.99% at checkpoint comparison over captured event windows.
4. Run opportunity detection + simulation within a strict per-block budget (target <2s p95 for candidate set).
5. Demonstrate relay submission dry-run path (no key material) with full observability traces.
6. Provide a one-command demo and a benchmark report that maps optimizations to measured wins.

---

### Task 1: Establish V2 Scope, Milestones, and KPI Contract

**Files:**
- Create: `docs/plans/v2_scope_kpi.md`
- Modify: `README.md`
- Modify: `mempool_mev_implementation.md`
- Test: `scripts/verify_phase_checks.sh`

**Step 1: Write the failing test**
- Add a verification expectation in `scripts/verify_phase_checks.sh` for existence of `docs/plans/v2_scope_kpi.md`.

**Step 2: Run test to verify it fails**
- Run: `./scripts/verify_phase_checks.sh`
- Expected: FAIL because V2 scope/KPI doc check is missing.

**Step 3: Write minimal implementation**
- Add a KPI contract doc with latency, throughput, determinism, and reliability SLOs.
- Update root `README.md` with V2 objectives and links.

**Step 4: Run test to verify it passes**
- Run: `./scripts/verify_phase_checks.sh`
- Expected: PASS for doc-check and existing build/test checks.

**Step 5: Commit**
```bash
git add docs/plans/v2_scope_kpi.md README.md mempool_mev_implementation.md scripts/verify_phase_checks.sh
git commit -m "docs: define v2 scope and measurable KPI contract"
```

---

### Task 2: Add Production devp2p Ingestion Lane (Primary, RPC as Fallback)

**Files:**
- Create: `crates/ingest/src/devp2p_runtime.rs`
- Create: `crates/ingest/tests/devp2p_runtime.rs`
- Modify: `crates/ingest/src/p2p.rs`
- Modify: `crates/ingest/src/lib.rs`
- Modify: `crates/viz-api/src/bin/viz-api.rs`
- Modify: `crates/viz-api/src/lib.rs`

**Step 1: Write the failing test**
- In `crates/ingest/tests/devp2p_runtime.rs`, add test asserting:
  - peer announcements produce `TxSeen`
  - fetch path produces `TxFetched` + `TxDecoded`
  - queue backpressure metrics are non-zero under stress.

**Step 2: Run test to verify it fails**
- Run: `cargo test -p ingest devp2p_runtime -- --nocapture`
- Expected: FAIL because runtime implementation does not exist.

**Step 3: Write minimal implementation**
- Implement a runtime wrapper that drives `P2pIngestService` with async peer inputs.
- Add source mode selection in `viz-api` boot path (`p2p`, `rpc`, `hybrid`).

**Step 4: Run test to verify it passes**
- Run: `cargo test -p ingest devp2p_runtime -- --nocapture`
- Expected: PASS.

**Step 5: Commit**
```bash
git add crates/ingest/src/devp2p_runtime.rs crates/ingest/tests/devp2p_runtime.rs crates/ingest/src/p2p.rs crates/ingest/src/lib.rs crates/viz-api/src/bin/viz-api.rs crates/viz-api/src/lib.rs
git commit -m "feat: add async devp2p ingestion lane and source-mode switch"
```

---

### Task 3: Expand Canonical Tx Schema for EVM-Meaningful Analysis

**Files:**
- Modify: `crates/event-log/src/lib.rs`
- Modify: `crates/storage/src/lib.rs`
- Modify: `crates/viz-api/src/live_rpc.rs`
- Modify: `crates/viz-api/src/lib.rs`
- Create: `crates/event-log/tests/tx_schema_roundtrip.rs`

**Step 1: Write the failing test**
- Add schema round-trip tests for enriched `TxDecoded` fields:
  - `to`, `value`, `gas_limit`, dynamic fee fields, calldata bytes length, chain_id.

**Step 2: Run test to verify it fails**
- Run: `cargo test -p event-log tx_schema_roundtrip`
- Expected: FAIL due missing fields.

**Step 3: Write minimal implementation**
- Extend `EventPayload::TxDecoded` model and storage DTOs.
- Update live ingest mapping and API projections.

**Step 4: Run test to verify it passes**
- Run: `cargo test -p event-log tx_schema_roundtrip`
- Expected: PASS.

**Step 5: Commit**
```bash
git add crates/event-log/src/lib.rs crates/storage/src/lib.rs crates/viz-api/src/live_rpc.rs crates/viz-api/src/lib.rs crates/event-log/tests/tx_schema_roundtrip.rs
git commit -m "feat: enrich canonical tx schema for evm-level analysis"
```

---

### Task 4: Build Deterministic EVM Simulation Crate for Candidate Evaluation

**Files:**
- Create: `crates/sim-engine/Cargo.toml`
- Create: `crates/sim-engine/src/lib.rs`
- Create: `crates/sim-engine/tests/deterministic_replay.rs`
- Modify: `Cargo.toml`
- Modify: `crates/replay/src/lib.rs`

**Step 1: Write the failing test**
- Add deterministic simulation test on same event window run twice with same state root and tx set.
- Assert identical outputs (gas used, status, state diff hash).

**Step 2: Run test to verify it fails**
- Run: `cargo test -p sim-engine deterministic_replay`
- Expected: FAIL because crate/functionality missing.

**Step 3: Write minimal implementation**
- Implement revm-backed simulation interface with deterministic config and explicit chain context.

**Step 4: Run test to verify it passes**
- Run: `cargo test -p sim-engine deterministic_replay`
- Expected: PASS.

**Step 5: Commit**
```bash
git add Cargo.toml crates/sim-engine/Cargo.toml crates/sim-engine/src/lib.rs crates/sim-engine/tests/deterministic_replay.rs crates/replay/src/lib.rs
git commit -m "feat: add deterministic revm simulation engine"
```

---

### Task 5: Implement MEV Opportunity Engine (Searcher Core)

**Files:**
- Create: `crates/searcher/Cargo.toml`
- Create: `crates/searcher/src/lib.rs`
- Create: `crates/searcher/src/strategies.rs`
- Create: `crates/searcher/tests/opportunity_scoring.rs`
- Modify: `Cargo.toml`
- Modify: `crates/feature-engine/src/lib.rs`

**Step 1: Write the failing test**
- Add test for candidate extraction + scoring from a mixed tx batch.
- Assert deterministic ranking and pruning behavior.

**Step 2: Run test to verify it fails**
- Run: `cargo test -p searcher opportunity_scoring`
- Expected: FAIL due missing crate.

**Step 3: Write minimal implementation**
- Build strategy trait and baseline strategies (`sandwich_candidate`, `backrun_candidate`, `arb_candidate`).
- Integrate feature engine outputs as searcher inputs.

**Step 4: Run test to verify it passes**
- Run: `cargo test -p searcher opportunity_scoring`
- Expected: PASS.

**Step 5: Commit**
```bash
git add Cargo.toml crates/searcher/Cargo.toml crates/searcher/src/lib.rs crates/searcher/src/strategies.rs crates/searcher/tests/opportunity_scoring.rs crates/feature-engine/src/lib.rs
git commit -m "feat: add mev opportunity engine with deterministic ranking"
```

---

### Task 6: Add PBS Relay/Builder Integration Dry-Run Path

**Files:**
- Create: `crates/builder/Cargo.toml`
- Create: `crates/builder/src/lib.rs`
- Create: `crates/builder/src/relay_client.rs`
- Create: `crates/builder/tests/relay_submission_dry_run.rs`
- Modify: `Cargo.toml`
- Modify: `crates/viz-api/src/lib.rs`

**Step 1: Write the failing test**
- Add test that builds a mock block payload and submits to a mocked relay endpoint.
- Assert structured retry/backoff and trace metadata.

**Step 2: Run test to verify it fails**
- Run: `cargo test -p builder relay_submission_dry_run`
- Expected: FAIL due missing crate.

**Step 3: Write minimal implementation**
- Implement relay client abstraction and dry-run submission path.
- Add API endpoint for dry-run status visibility.

**Step 4: Run test to verify it passes**
- Run: `cargo test -p builder relay_submission_dry_run`
- Expected: PASS.

**Step 5: Commit**
```bash
git add Cargo.toml crates/builder/Cargo.toml crates/builder/src/lib.rs crates/builder/src/relay_client.rs crates/builder/tests/relay_submission_dry_run.rs crates/viz-api/src/lib.rs
git commit -m "feat: add pbs relay dry-run integration"
```

---

### Task 7: Add Performance Profiling and Latency Budgets (Hot Path Hardening)

**Files:**
- Create: `crates/bench/Cargo.toml`
- Create: `crates/bench/benches/pipeline_latency.rs`
- Create: `scripts/profile_pipeline.sh`
- Create: `docs/perf/v2_baseline.md`
- Modify: `Cargo.toml`
- Modify: `README.md`

**Step 1: Write the failing test**
- Add criterion benchmark expectation script that fails when p95 latency exceeds threshold.

**Step 2: Run test to verify it fails**
- Run: `bash scripts/profile_pipeline.sh --check-budget`
- Expected: FAIL against strict placeholder threshold.

**Step 3: Write minimal implementation**
- Add benchmark harness for ingest -> decode -> feature -> candidate scoring.
- Add flamegraph generation and allocator/runtime toggle matrix.

**Step 4: Run test to verify it passes**
- Run: `bash scripts/profile_pipeline.sh --check-budget`
- Expected: PASS with measured output and generated artifacts.

**Step 5: Commit**
```bash
git add Cargo.toml crates/bench/Cargo.toml crates/bench/benches/pipeline_latency.rs scripts/profile_pipeline.sh docs/perf/v2_baseline.md README.md
git commit -m "perf: add benchmark and latency budget enforcement"
```

---

### Task 8: Production Observability and Alerting Pack

**Files:**
- Create: `ops/prometheus/prometheus.yml`
- Create: `ops/prometheus/alerts.yml`
- Create: `ops/grafana/dashboards/v2_overview.json`
- Create: `docs/runbooks/incident_response.md`
- Modify: `crates/common/src/lib.rs`
- Modify: `crates/viz-api/src/lib.rs`

**Step 1: Write the failing test**
- Add alert rule tests for queue saturation, ingest lag, decode failures, coverage collapse.

**Step 2: Run test to verify it fails**
- Run: `cargo test -p common evaluate_alerts`
- Expected: FAIL for newly added rule cases.

**Step 3: Write minimal implementation**
- Extend alert evaluation model and expose metrics endpoints.
- Add Prometheus scrape + alert rule configs and dashboard.

**Step 4: Run test to verify it passes**
- Run: `cargo test -p common evaluate_alerts`
- Expected: PASS.

**Step 5: Commit**
```bash
git add ops/prometheus/prometheus.yml ops/prometheus/alerts.yml ops/grafana/dashboards/v2_overview.json docs/runbooks/incident_response.md crates/common/src/lib.rs crates/viz-api/src/lib.rs
git commit -m "ops: add production observability and alert pack"
```

---

### Task 9: Historical Storage, Backfill, and Retention Strategy

**Files:**
- Create: `crates/storage/src/clickhouse_schema.rs`
- Create: `crates/storage/src/backfill.rs`
- Create: `crates/storage/tests/backfill.rs`
- Create: `docker/clickhouse/docker-compose.yml`
- Modify: `crates/storage/src/lib.rs`

**Step 1: Write the failing test**
- Add backfill idempotency test and retention-window pruning test.

**Step 2: Run test to verify it fails**
- Run: `cargo test -p storage backfill`
- Expected: FAIL before implementation.

**Step 3: Write minimal implementation**
- Add DDL helpers, partition keys, TTL policy, and backfill/replay writer.

**Step 4: Run test to verify it passes**
- Run: `cargo test -p storage backfill`
- Expected: PASS.

**Step 5: Commit**
```bash
git add crates/storage/src/clickhouse_schema.rs crates/storage/src/backfill.rs crates/storage/tests/backfill.rs docker/clickhouse/docker-compose.yml crates/storage/src/lib.rs
git commit -m "feat: add clickhouse schema and idempotent backfill pipeline"
```

---

### Task 10: End-to-End Interview Demo Scenario and Reproducibility

**Files:**
- Create: `scripts/demo_v2.sh`
- Create: `docs/demo/interview_demo.md`
- Create: `docs/demo/sample_output.md`
- Modify: `apps/web-ui/README.md`
- Modify: `README.md`

**Step 1: Write the failing test**
- Add shell checks in `scripts/demo_v2.sh` for service readiness + endpoint output shape.

**Step 2: Run test to verify it fails**
- Run: `bash scripts/demo_v2.sh --verify`
- Expected: FAIL until script and docs are complete.

**Step 3: Write minimal implementation**
- One-command launch for API/UI/metrics.
- Include scripted narrative:
  - live ingest
  - replay determinism
  - opportunity ranking
  - relay dry-run
  - dashboard interpretation.

**Step 4: Run test to verify it passes**
- Run: `bash scripts/demo_v2.sh --verify`
- Expected: PASS with reproducible output.

**Step 5: Commit**
```bash
git add scripts/demo_v2.sh docs/demo/interview_demo.md docs/demo/sample_output.md apps/web-ui/README.md README.md
git commit -m "docs: add reproducible v2 interview demo workflow"
```

---

## Global Verification Gate (Required Before Merge)

Run all commands:

```bash
cargo test --workspace
cargo build --workspace
./scripts/verify_phase_checks.sh
bash scripts/profile_pipeline.sh --check-budget
bash scripts/demo_v2.sh --verify
```

Expected:
- All tests pass.
- Build succeeds across all crates.
- Latency budget check passes.
- Demo verification passes with deterministic output.

## Interview Artifact Checklist

1. Architecture diagram with ingest/searcher/builder boundaries.
2. Performance report with before/after optimization deltas.
3. Deterministic replay parity report.
4. MEV opportunity backtest summary (precision/recall or PnL proxy).
5. Relay integration dry-run logs and failure handling traces.
6. 10-minute live demo script and backup prerecorded output.

## Risk Register

1. **Public RPC instability**
- Mitigation: prioritize devp2p mode + multi-provider fallback.

2. **Non-deterministic simulation outputs**
- Mitigation: strict chain context pinning and deterministic seeds.

3. **Latency regression from richer features**
- Mitigation: budgeted profiling gate in CI.

4. **Scope creep into full builder product**
- Mitigation: keep relay path dry-run and interview-focused.

## Suggested Timeline (6 Weeks)

1. Week 1: Tasks 1-2 (scope + devp2p lane)
2. Week 2: Tasks 3-4 (schema + deterministic simulation)
3. Week 3: Task 5 (searcher core)
4. Week 4: Task 6 (relay dry-run) + Task 7 (performance)
5. Week 5: Tasks 8-9 (ops + storage/backfill)
6. Week 6: Task 10 + final verification + interview artifact polish

