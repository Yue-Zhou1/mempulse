# Commercial Readiness Improvement Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Harden the V2 mempool/MEV stack from demo-grade into commercially viable infrastructure by closing the highest-impact readiness gaps (strategy generality, simulation fidelity, storage durability, and product/API hardening).

**Architecture:** Keep the existing event-driven pipeline and deterministic replay backbone, but replace hardcoded heuristics with config-driven registries and explainable scoring, introduce higher-fidelity simulation modes, and add durable storage + product-grade API controls. Preserve deterministic regression checks and extend verification scripts so commercial claims are measurable and repeatable.

**Tech Stack:** Rust (Tokio, Axum), revm, serde/JSON config, ClickHouse, Parquet (Arrow/Parquet crate), Prometheus/Grafana, bash verification scripts.

## Scope Assumptions

- Keep the current repo layout and crate boundaries.
- Land all changes behind defaults that preserve current demo behavior.
- Prioritize additive schema/API changes with backward compatibility where possible.

---

### Task 1: Normalize Ingest Runtime Configuration (Remove Hardcoded Endpoint Drift)

**Files:**
- Create: `crates/viz-api/tests/live_rpc_env_config.rs`
- Modify: `crates/viz-api/src/live_rpc.rs`
- Modify: `crates/viz-api/src/lib.rs`
- Modify: `README.md`

**Step 1: Write the failing test**

```rust
#[test]
fn live_rpc_config_prefers_env_over_defaults() {
    std::env::set_var("VIZ_API_ETH_WS_URL", "wss://example/ws");
    std::env::set_var("VIZ_API_ETH_HTTP_URL", "https://example/http");
    std::env::set_var("VIZ_API_SOURCE_ID", "rpc-custom");
    let cfg = viz_api::live_rpc::LiveRpcConfig::from_env().expect("config");
    assert_eq!(cfg.primary().ws_url(), "wss://example/ws");
    assert_eq!(cfg.primary().http_url(), "https://example/http");
    assert_eq!(cfg.source_id().to_string(), "rpc-custom");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api live_rpc_config_prefers_env_over_defaults -- --nocapture`  
Expected: FAIL because `from_env`/accessor methods do not exist yet.

**Step 3: Write minimal implementation**

```rust
impl LiveRpcConfig {
    pub fn from_env() -> anyhow::Result<Self> { /* parse env or fallback defaults */ }
}
```

Wire `default_state()` to `LiveRpcConfig::from_env()` so docs and runtime behavior match.

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api live_rpc_config_prefers_env_over_defaults -- --nocapture`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/tests/live_rpc_env_config.rs crates/viz-api/src/live_rpc.rs crates/viz-api/src/lib.rs README.md
git commit -m "fix: make live rpc ingest endpoints fully env-configurable"
```

---

### Task 2: Replace Hardcoded Protocol Heuristics with Config-Driven Registry

**Files:**
- Create: `configs/protocol_registry.mainnet.json`
- Create: `crates/feature-engine/src/registry.rs`
- Create: `crates/feature-engine/tests/protocol_registry.rs`
- Modify: `crates/feature-engine/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn classifies_protocol_from_registry_json_without_code_changes() {
    let registry = ProtocolRegistry::from_json(r#"{
      "protocols":[{"name":"curve-v2","addresses":["0x1212...1212"]}]
    }"#).unwrap();
    let protocol = registry.classify(Some([0x12; 20]), Some([0xde,0xad,0xbe,0xef]));
    assert_eq!(protocol, "curve-v2");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p feature-engine classifies_protocol_from_registry_json_without_code_changes -- --nocapture`  
Expected: FAIL because no registry loader/classifier exists.

**Step 3: Write minimal implementation**

```rust
pub struct ProtocolRegistry { /* address + selector maps */ }
impl ProtocolRegistry {
    pub fn from_json(input: &str) -> anyhow::Result<Self> { /* serde parse */ }
    pub fn classify(&self, to: Option<Address>, selector: Option<[u8;4]>) -> &str { /* lookup */ }
}
```

Refactor `analyze_transaction` to use `ProtocolRegistry::default_mainnet()` instead of only hardcoded constants.

**Step 4: Run test to verify it passes**

Run: `cargo test -p feature-engine protocol_registry -- --nocapture`  
Expected: PASS.

**Step 5: Commit**

```bash
git add configs/protocol_registry.mainnet.json crates/feature-engine/src/registry.rs crates/feature-engine/tests/protocol_registry.rs crates/feature-engine/src/lib.rs
git commit -m "feat: add config-driven protocol registry for feature classification"
```

---

### Task 3: Add Explainable, Calibratable Searcher Scoring

**Files:**
- Create: `crates/searcher/src/scoring.rs`
- Create: `crates/searcher/tests/explainable_scoring.rs`
- Modify: `crates/searcher/src/lib.rs`
- Modify: `crates/searcher/src/strategies.rs`
- Modify: `crates/searcher/examples/backtest_summary.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn ranked_candidates_include_score_breakdown_and_reasons() {
    let ranked = rank_opportunities(&sample_batch(), SearcherConfig::default());
    let top = ranked.first().unwrap();
    assert!(!top.reasons.is_empty());
    assert!(top.reasons.iter().any(|r| r.contains("mev_score")));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p searcher ranked_candidates_include_score_breakdown_and_reasons -- --nocapture`  
Expected: FAIL because `OpportunityCandidate` has no explanation payload.

**Step 3: Write minimal implementation**

```rust
pub struct ScoreBreakdown { pub mev_component: u32, pub urgency_component: u32, pub bonus_component: u32 }
pub struct OpportunityCandidate { /* existing fields */ pub breakdown: ScoreBreakdown, pub reasons: Vec<String> }
```

Update strategy evaluators to emit deterministic reason strings and breakdown values.

**Step 4: Run test to verify it passes**

Run: `cargo test -p searcher explainable_scoring opportunity_scoring -- --nocapture`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/searcher/src/scoring.rs crates/searcher/tests/explainable_scoring.rs crates/searcher/src/lib.rs crates/searcher/src/strategies.rs crates/searcher/examples/backtest_summary.rs
git commit -m "feat: add explainable scoring outputs for searcher candidates"
```

---

### Task 4: Introduce Higher-Fidelity Simulation Mode (RPC-Backed State + Real Input Bytes)

**Files:**
- Create: `crates/sim-engine/src/state_provider.rs`
- Create: `crates/sim-engine/tests/rpc_backed_mode.rs`
- Modify: `crates/sim-engine/src/lib.rs`
- Modify: `crates/event-log/src/lib.rs`
- Modify: `crates/viz-api/src/live_rpc.rs`
- Modify: `crates/storage/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn rpc_backed_mode_fetches_account_state_before_execution() {
    let provider = MockStateProvider::with_account(/* sender */);
    let result = simulate_with_mode(ctx(), &[decoded_with_calldata()], SimulationMode::RpcBacked(provider)).await.unwrap();
    assert_eq!(result.tx_results.len(), 1);
    assert!(provider.account_calls() > 0);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sim-engine rpc_backed_mode_fetches_account_state_before_execution -- --nocapture`  
Expected: FAIL because `SimulationMode` and state provider interfaces are missing.

**Step 3: Write minimal implementation**

```rust
pub enum SimulationMode { SyntheticDeterministic, RpcBacked(Arc<dyn StateProvider>) }
pub trait StateProvider { async fn account_info(&self, address: Address) -> anyhow::Result<AccountSeed>; }
```

Add optional calldata bytes to decoded/storage records, and feed real calldata into `TxEnv.data` when available.

**Step 4: Run test to verify it passes**

Run: `cargo test -p sim-engine deterministic_replay rpc_backed_mode -- --nocapture`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/sim-engine/src/state_provider.rs crates/sim-engine/tests/rpc_backed_mode.rs crates/sim-engine/src/lib.rs crates/event-log/src/lib.rs crates/viz-api/src/live_rpc.rs crates/storage/src/lib.rs
git commit -m "feat: add rpc-backed simulation mode with higher input/state fidelity"
```

---

### Task 5: Add Durable Storage Path and Production Export Capability

**Files:**
- Create: `crates/storage/src/wal.rs`
- Create: `crates/storage/tests/wal_recovery.rs`
- Create: `crates/storage/tests/parquet_export.rs`
- Modify: `crates/storage/src/lib.rs`
- Modify: `crates/storage/Cargo.toml`

**Step 1: Write the failing tests**

```rust
#[test]
fn writer_recovers_unflushed_events_from_wal_on_restart() { /* enqueue -> crash -> restart -> recovered */ }

#[test]
fn parquet_export_writes_replay_frames_that_can_be_read_back() { /* write parquet -> read rows */ }
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p storage wal_recovery parquet_export -- --nocapture`  
Expected: FAIL because WAL and Parquet implementations do not exist.

**Step 3: Write minimal implementation**

```rust
pub struct StorageWal { /* append/read/ack */ }
pub struct ArrowParquetExporter;
impl ParquetExporter for ArrowParquetExporter { /* write frames */ }
```

Integrate optional WAL into `spawn_single_writer` and replace `UnsupportedParquetExporter`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p storage backfill wal_recovery parquet_export -- --nocapture`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/storage/src/wal.rs crates/storage/tests/wal_recovery.rs crates/storage/tests/parquet_export.rs crates/storage/src/lib.rs crates/storage/Cargo.toml
git commit -m "feat: add WAL-backed durability and parquet replay export"
```

---

### Task 6: Harden API for Commercial Multi-Client Use (Auth, Quotas, Tenant Isolation)

**Files:**
- Create: `crates/viz-api/src/auth.rs`
- Create: `crates/viz-api/tests/api_auth_and_limits.rs`
- Modify: `crates/viz-api/src/lib.rs`
- Modify: `crates/viz-api/Cargo.toml`
- Modify: `docs/runbooks/incident_response.md`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn protected_routes_require_valid_api_key() {
    let app = build_router(test_state());
    let res = app.oneshot(Request::builder().uri("/transactions").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p viz-api protected_routes_require_valid_api_key -- --nocapture`  
Expected: FAIL because middleware/auth is not present.

**Step 3: Write minimal implementation**

```rust
pub struct ApiAuthConfig { pub enabled: bool, pub api_keys: HashSet<String> }
// Axum middleware validates `x-api-key` and enforces per-key token bucket.
```

Apply middleware to non-health routes and add per-tenant request counters.

**Step 4: Run test to verify it passes**

Run: `cargo test -p viz-api api_auth_and_limits -- --nocapture`  
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/viz-api/src/auth.rs crates/viz-api/tests/api_auth_and_limits.rs crates/viz-api/src/lib.rs crates/viz-api/Cargo.toml docs/runbooks/incident_response.md
git commit -m "feat: add api key auth and quota controls to viz-api"
```

---

### Task 7: Add Commercial Readiness Verification Gate

**Files:**
- Create: `scripts/verify_commercial_readiness.sh`
- Modify: `scripts/verify_phase_checks.sh`
- Modify: `docs/plans/v2_scope_kpi.md`
- Create: `docs/reports/commercial_readiness_report.md`

**Step 1: Write the failing gate**

Add checks for:
- explainable candidate outputs,
- rpc-backed simulation smoke,
- WAL recovery test,
- auth-required API route test.

**Step 2: Run to verify it fails**

Run: `bash scripts/verify_commercial_readiness.sh`  
Expected: FAIL until Tasks 1-6 are implemented.

**Step 3: Write minimal implementation**

```bash
cargo test -p searcher explainable_scoring -- --nocapture
cargo test -p sim-engine rpc_backed_mode -- --nocapture
cargo test -p storage wal_recovery parquet_export -- --nocapture
cargo test -p viz-api api_auth_and_limits -- --nocapture
```

Wire this script into `scripts/verify_phase_checks.sh` as an optional or required stage.

**Step 4: Run gate to verify it passes**

Run: `bash scripts/verify_commercial_readiness.sh`  
Expected: PASS with explicit per-stage PASS output.

**Step 5: Commit**

```bash
git add scripts/verify_commercial_readiness.sh scripts/verify_phase_checks.sh docs/plans/v2_scope_kpi.md docs/reports/commercial_readiness_report.md
git commit -m "chore: add commercial readiness verification gate and reporting"
```

---

## Execution Order and Risk Burn-Down

1. Task 1 (config correctness)  
2. Task 2 (feature generalization)  
3. Task 3 (scoring explainability)  
4. Task 4 (simulation fidelity)  
5. Task 5 (storage durability/export)  
6. Task 6 (API commercialization controls)  
7. Task 7 (gated verification)

This order reduces go-to-market risk fastest while keeping existing demo workflows operational.
