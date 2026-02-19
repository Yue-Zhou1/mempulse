# prototype03

## Workspace Layout

This repository is a Rust workspace for the mempool/MEV system.

- `crates/common` shared domain types and utility primitives
- `crates/event-log` canonical append-only event contracts
- `crates/rpc-ingest` RPC pending transaction ingestion pipeline
- `crates/p2p-ingest` devp2p ingestion pipeline (planned)
- `crates/tx-decode` transaction decoding and normalization
- `crates/mempool-state` lifecycle/replacement/reorg-aware state
- `crates/feature-engine` MEV-oriented feature derivation
- `crates/storage` storage adapters and persistence services
- `crates/viz-api` API for observability/replay visualization clients
- `crates/replay` deterministic replay capabilities
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
