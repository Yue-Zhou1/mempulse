# prototype03

## Workspace Layout

This repository is a Rust workspace for the mempool/MEV system.

- `crates/common` shared domain types and utility primitives
- `crates/event-log` canonical append-only event contracts
- `crates/ingest` transaction decode + RPC/devp2p ingestion pipelines
- `crates/feature-engine` MEV-oriented feature derivation
- `crates/storage` storage adapters and persistence services
- `crates/viz-api` API for observability/replay visualization clients
- `crates/replay` deterministic replay and lifecycle/reorg-aware state
- `crates/bench` latency benchmark harness and profiling helpers
- `apps/web-ui` Magic UI-style 2D frontend for replay/analysis

## Operational Metrics Model

Current metrics and alert-threshold primitives are defined in `crates/common`:

- peer/session churn
- ingest lag
- decode failure rate
- coverage collapse
- storage write latency
- clock skew

The p2p ingest pipeline exposes queue/backpressure telemetry:

- `queue_depth_current`
- `queue_depth_peak`
- `queue_dropped_total`
- `duplicates_dropped_total`

## Verification Workflow

Run the full acceptance verification command:

```bash
./scripts/verify_phase_checks.sh
```

This command runs:

- `cargo test --workspace`
- `cargo build -p replay --bin replay-cli`
- `cargo build -p viz-api --bin viz-api`

## V2 Implementation Docs

The demo-focused V2 plan and KPI contract are documented at:

- `docs/plans/2026-02-21-mev-infra-v2-implementation.md`
- `docs/plans/v2_scope_kpi.md`
- `docs/perf/v2_baseline.md`

The legacy implementation plan now lives at:

- `docs/mempool_mev_implementation.md`

## Real Mempool Mode

`viz-api` can ingest real pending transactions from an Ethereum RPC provider.

Set environment variables before starting `viz-api`:

- `VIZ_API_ETH_WS_URL` (optional override): WebSocket endpoint for `eth_subscribe newPendingTransactions`
- `VIZ_API_ETH_HTTP_URL` (optional override): HTTP endpoint for `eth_getTransactionByHash`
- `VIZ_API_SOURCE_ID` (optional): source label (default: `rpc-live`)
- `VIZ_API_MAX_SEEN_HASHES` (optional): dedup cache size for live feed (default: `10000`)

Example:

```bash
export VIZ_API_ETH_WS_URL="wss://YOUR_PROVIDER_WS_URL"
export VIZ_API_ETH_HTTP_URL="https://YOUR_PROVIDER_HTTP_URL"
cargo run -p viz-api --bin viz-api
```

Without these overrides, `viz-api` uses built-in public RPC endpoints for live mode.

## Performance Budget Check

Run the pipeline latency budget check and generate baseline artifacts:

```bash
bash scripts/profile_pipeline.sh --check-budget
```

Artifacts are written to:

- `target/perf/pipeline_latency_summary.json`
- `target/perf/runtime_allocator_matrix.tsv`
- `target/perf/pipeline_latency_snapshot.log`
- `docs/perf/v2_baseline.md`

## V2 Demo Workflow

Verification:

```bash
bash scripts/demo_v2.sh --verify
```

Run API + UI together:

```bash
bash scripts/demo_v2.sh --run
```

Reference docs:

- `docs/demo/v2_demo.md`
- `docs/demo/sample_output.md`
