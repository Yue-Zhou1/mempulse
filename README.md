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
- `VIZ_API_CHAINS` (optional): JSON array for chain-scoped ingest workers (overrides single-chain vars)
- `VIZ_API_MAX_SEEN_HASHES` (optional): dedup cache size for live feed (default: `10000`)
- `VIZ_API_RPC_BATCH_SIZE` (optional): hashes per `eth_getTransactionByHash` batch request (default: `32`)
- `VIZ_API_RPC_MAX_IN_FLIGHT` (optional): per-chain in-flight RPC batch cap (default: `4`)
- `VIZ_API_RPC_RETRY_ATTEMPTS` (optional): retry count after initial batch request failure (default: `2`)
- `VIZ_API_RPC_RETRY_BACKOFF_MS` (optional): base retry backoff in milliseconds (default: `100`)
- `VIZ_API_RPC_BATCH_FLUSH_MS` (optional): max wait before flushing queued hashes into batch fetch (default: `40`)

Example:

```bash
export VIZ_API_ETH_WS_URL="wss://YOUR_PROVIDER_WS_URL"
export VIZ_API_ETH_HTTP_URL="https://YOUR_PROVIDER_HTTP_URL"
export VIZ_API_RPC_BATCH_SIZE="32"
export VIZ_API_RPC_MAX_IN_FLIGHT="4"
cargo run -p viz-api --bin viz-api
```

Multi-chain example:

```bash
export VIZ_API_CHAINS='[
  {
    "chain_key": "eth-mainnet",
    "chain_id": 1,
    "ws_url": "wss://YOUR_ETH_WS_URL",
    "http_url": "https://YOUR_ETH_HTTP_URL",
    "source_id": "rpc-eth-mainnet"
  },
  {
    "chain_key": "base-mainnet",
    "chain_id": 8453,
    "ws_url": "wss://YOUR_BASE_WS_URL",
    "http_url": "https://YOUR_BASE_HTTP_URL",
    "source_id": "rpc-base-mainnet"
  }
]'
cargo run -p viz-api --bin viz-api
```

Without these overrides, `viz-api` uses built-in public RPC endpoints for live mode.

### Chain-Aware API Filters

The following endpoints accept an optional `chain_id` query parameter:

- `/dashboard/snapshot`
- `/transactions`
- `/transactions/all`
- `/features/recent`
- `/opps/recent`

Examples:

```bash
curl "http://127.0.0.1:3000/transactions?chain_id=1&limit=50"
curl "http://127.0.0.1:3000/dashboard/snapshot?chain_id=8453&tx_limit=100"
```

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

## Transaction Pipeline Baseline Harness

Run the deterministic transaction ingest/read-path baseline harness:

```bash
bash scripts/perf_tx_pipeline_baseline.sh
```

Optional environment variables:

- `VIZ_API_TX_PERF_TX_COUNT` to override seeded transaction volume (default: `2000`)
- `VIZ_API_TX_PERF_ARTIFACT` to write to a specific artifact path
- `TX_PIPELINE_PERF_ARTIFACT_DIR` to override artifact directory (default: `artifacts/perf`)

Artifacts are written to:

- `artifacts/perf/tx_pipeline_perf_baseline_<timestamp>.json`
- `artifacts/perf/tx_pipeline_perf_baseline_latest.json`

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
