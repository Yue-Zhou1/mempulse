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
- `apps/web-ui` custom frontend skeleton (Three.js-ready)

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

## Real Mempool Mode

`viz-api` can ingest real pending transactions from an Ethereum RPC provider.

Set environment variables before starting `viz-api`:

- `VIZ_API_ETH_WS_URL` (required): WebSocket endpoint for `eth_subscribe newPendingTransactions`
- `VIZ_API_ETH_HTTP_URL` (recommended): HTTP endpoint for `eth_getTransactionByHash`
- `VIZ_API_SOURCE_ID` (optional): source label (default: `rpc-live`)

Example:

```bash
export VIZ_API_ETH_WS_URL="wss://YOUR_PROVIDER_WS_URL"
export VIZ_API_ETH_HTTP_URL="https://YOUR_PROVIDER_HTTP_URL"
cargo run -p viz-api --bin viz-api
```

Without these variables, `viz-api` falls back to synthetic demo events.
