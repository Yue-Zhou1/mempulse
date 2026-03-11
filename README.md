# Mempulse

Mempulse is a Rust + React workspace for ingesting live mempool traffic, deriving MEV-oriented features, running scheduler/search/simulation/builder workflows, persisting runtime state, and serving replay plus dashboard APIs.

## Current Architecture

1. `ingest` normalizes pending transactions from RPC and devp2p sources into canonical event-log envelopes.
2. `runtime-core` orchestrates ingest, storage, scheduler, searcher, simulation, and builder components.
3. `storage` persists WAL-backed events, bounded read models, and scheduler snapshots, while `replay` reconstructs lifecycle state deterministically.
4. `viz-api` exposes HTTP and SSE endpoints, and `apps/web-ui` consumes `snapshot-v2` plus `events-v1`.

## Real-World Use Cases

In practical terms, this repo can be used as the backbone for a mempool intelligence and runtime-analysis stack. It can watch live pending transaction flow, classify and score what is happening, persist the event stream for later replay, and expose that state through APIs and a dashboard so operators or researchers can inspect the system in real time.

- Run a live mempool monitoring service across one or more EVM chains using RPC or devp2p ingest.
- Investigate incidents such as dropped transactions, replay mismatches, reorg effects, queue saturation, or latency regressions using persisted events, replay, metrics, and alerts.
- Build internal dashboards for pending transaction flow, derived protocol features, opportunity scoring, relay dry-run state, and per-chain ingest health.
- Prototype and evaluate searcher, simulation, scheduler, and builder logic before integrating those components into a larger production trading or infra stack.

What it is not:

- This repo is not a turnkey profit-generating bot by itself; it is the infrastructure layer for observing, testing, and operating mempool-aware systems.

## Workspace Layout

- `crates/common`: shared IDs, alert primitives, metrics helpers, and domain types
- `crates/event-log`: canonical append-only event contracts
- `crates/ingest`: RPC and devp2p ingestion, pending transaction fetch, and decode logic
- `crates/feature-engine`: protocol classification and MEV feature derivation
- `crates/scheduler`: pending queueing, nonce-gap handling, replacement policy, and simulation handoff
- `crates/searcher`: opportunity scoring and explainable ranking output
- `crates/sim-engine`: deterministic and RPC-backed execution simulation
- `crates/builder`: candidate assembly and relay dry-run packaging
- `crates/storage`: in-memory projections, WAL recovery, parquet export, ClickHouse adapters, and scheduler snapshot persistence
- `crates/replay`: lifecycle replay, checkpoint parity, and replay diff summaries
- `crates/runtime-core`: long-lived runtime orchestration and live RPC workers
- `crates/node-runtime`: runtime bootstrap and lifecycle wrapper used by the API binary
- `crates/viz-api`: Axum API, SSE transport, metrics, replay, and dashboard endpoints
- `crates/bench`: perf harnesses for tx pipeline, scheduler pipeline, simulation roundtrip, and storage snapshot
- `apps/web-ui`: React 19 + Vite dashboard client

## Running Locally

Prerequisites:

- Rust `1.88.0`
- Node `20+`
- `npm`

Full-stack demo commands:

```bash
bash scripts/demo_v2.sh --verify
bash scripts/demo_v2.sh --run
bash scripts/demo_v2.sh --run --prod
```

Notes:

- `bash scripts/demo_v2.sh --run` starts `viz-api` plus the Vite dev server.
- `bash scripts/demo_v2.sh --run --prod` starts `viz-api` plus a production-built web UI preview.
- Default ports are API `3100` and UI `5174`.
- Demo logs are written to `target/demo/viz-api.log` and `target/demo/web-ui.log`.

Run the backend only:

```bash
cargo run -p viz-api --bin viz-api
```

## Verification and CI

These commands mirror the current local/CI verification flow:

```bash
bash scripts/verify_static_checks.sh
bash scripts/verify_phase_checks.sh
cargo bench -p bench --no-run
bash scripts/perf_regression_check.sh
cd apps/web-ui && npm ci && npm test && npm run build
```

## API Surface

Current primary endpoints:

- `/health`
- `/metrics`
- `/metrics/snapshot`
- `/alerts/evaluate`
- `/dashboard/snapshot-v2`
- `/dashboard/events-v1`
- `/transactions`
- `/transactions/all`
- `/transactions/{hash}`
- `/features`
- `/features/recent`
- `/opps`
- `/opps/recent`
- `/replay`
- `/propagation`
- `/relay/dry-run/status`

## Real Mempool and Chain Configuration

`viz-api` can ingest live pending transactions from Ethereum-compatible RPC providers.

Default configuration is loaded from `configs/chain_config.json`. Override it with:

- `VIZ_API_CHAIN_CONFIG_PATH`: custom chain config JSON path
- `VIZ_API_CHAINS`: JSON chain list override, takes priority over file config
- `VIZ_API_ETH_WS_URL`: single-chain WebSocket override for the primary chain
- `VIZ_API_ETH_HTTP_URL`: single-chain HTTP override for the primary chain
- `VIZ_API_SOURCE_ID`: single-chain source label override for the primary chain
- `VIZ_API_MAX_SEEN_HASHES`: live-feed dedup cache size, default `10000`
- `VIZ_API_RPC_BATCH_SIZE`: hashes per batch request, default `32`
- `VIZ_API_RPC_MAX_IN_FLIGHT`: per-chain in-flight RPC batch cap, default `4`
- `VIZ_API_RPC_RETRY_ATTEMPTS`: retry count after the initial batch request, default `2`
- `VIZ_API_RPC_RETRY_BACKOFF_MS`: base retry backoff in milliseconds, default `100`
- `VIZ_API_RPC_BATCH_FLUSH_MS`: max wait before flushing queued hashes, default `40`
- `VIZ_API_SILENT_CHAIN_TIMEOUT_SECS`: rotate to the next endpoint after this many silent seconds, default `20`

Endpoints that accept an optional `chain_id` filter:

- `/dashboard/snapshot-v2`
- `/transactions`
- `/transactions/all`
- `/features/recent`
- `/opps/recent`

`/dashboard/snapshot-v2` also returns `chain_ingest_status` so the UI can render per-chain worker state.

## Performance Tooling

Run the pipeline budget check:

```bash
bash scripts/profile_pipeline.sh --check-budget
```

Run the deterministic tx pipeline baseline harness:

```bash
bash scripts/perf_tx_pipeline_baseline.sh
```

Run the full perf regression gate:

```bash
bash scripts/perf_regression_check.sh
```

Perf artifacts are written under `artifacts/perf/`.

## Reference Docs

- `docs/demo/v2_demo.md`
- `docs/demo/sample_output.md`
- `docs/plans/v2_scope_kpi.md`
- `docs/plans/2026-02-21-mev-infra-v2-implementation.md`
- `docs/architecture/v2_architecture_diagram.md`
- `docs/perf/v2_baseline.md`
- `docs/mempool_mev_implementation.md`
