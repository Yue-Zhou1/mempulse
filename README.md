# Mempulse

Mempulse is a Rust + React workspace for ingesting live mempool traffic, deriving MEV-oriented features, running scheduler/search/simulation/builder workflows, persisting runtime state, and serving replay plus dashboard APIs.

## Architecture

```
    External sources
    +----------------------+      +----------------------+
    | EVM RPC endpoints    |      | devp2p peers         |
    | WS + HTTP pending tx |      | announces / pooled tx|
    +----------+-----------+      +-----------+----------+
               \                              /
                \                            /
                 +-----------v--------------+
                 |       runtime-core       |  crates/runtime-core
                 | live ingest + pipeline   |  - owns live tx intake
                 | orchestration            |  - coordinates scheduler,
                 |                          |    feature/search/sim/builder
                 +-----+---------------+----+  - exposes runtime metrics/views
                       |               |
           event-log   |               | live snapshots
           envelopes   |               |
                       v               v
              +--------+-----+   +-----+------+
              | storage       |   |  viz-api   |  crates/viz-api
              | + replay      |   |  Axum + SSE|  - storage-backed reads
              | projections   |   |            |  - runtime-core live views
              +--------+-----+   +-----+------+
                       ^               |
                       |               |
                +------+-----+   +-----+------+
                | event-log   |   |  web-ui    |  apps/web-ui
                | + common    |   | React/Vite |
                +------------+   +------------+

    Library crates used by the live runtime:
    [ ingest ] [ scheduler ] [ feature-engine ] [ searcher ] [ sim-engine ] [ builder ]

    Bootstrap / lifecycle wrapper:
    node-runtime (crates/node-runtime) starts runtime-core and manages shutdown hooks
```

**Current live pipeline inside `runtime-core`:**
```
      +-----------------------+
      | ingress normalization |
      +-----------+-----------+
                  |
      +-----------v-----------+
      | scheduler admission   |
      | + nonce-gap tracking  |
      +-----------+-----------+
                  |
      +-----------v-----------+
      | feature-engine        |
      | protocol + scoring    |
      +-----------+-----------+
                  |
      +-----------v-----------+
      | searcher ranking      |
      | + bundle synthesis    |
      +-----------+-----------+
                  |
      +-----------v-----------+
      | sim-engine execution  |
      +-----------+-----------+
                  |
      +-----------v-----------+
      | builder assembly      |
      | + dry-run utilities   |
      +-----------------------+
```

**Data flow summary:**
1. `node-runtime` is the thin process wrapper: it starts `runtime-core`, applies ingest mode selection, and manages shutdown hooks.
2. `runtime-core` owns the current live pipeline. It subscribes to pending transactions over RPC WebSocket (`eth_subscribe:newPendingTransactions`), fetches transaction payloads over HTTP by hash or from the pending block, normalizes observations into canonical `event-log` envelopes, and tracks live runtime metrics and status views.
3. `ingest` contains reusable RPC/devp2p ingest services and tx decoding helpers. The current live runtime uses those concepts, while the main orchestration and live RPC helpers live in `runtime-core`.
4. `scheduler` admits transactions with dedup and fee-bump replacement, tracks per-sender nonce gaps and execution order, and marks transactions ready or blocked for downstream work.
5. `feature-engine` classifies protocols and computes MEV/urgency scores; `searcher` ranks opportunities and synthesizes bundle candidates from adjacent nonces.
6. `sim-engine` executes candidates in deterministic or RPC-backed modes. Accepted simulation results feed `builder` assembly state; the builder crate currently provides assembly decisions and relay dry-run utilities rather than confirmed live relay submission.
7. `storage` persists events to the WAL and optional ClickHouse sink while maintaining bounded in-memory projections; `replay` reconstructs lifecycle state and checkpoint parity from the stored event stream.
8. `viz-api` serves HTTP + SSE endpoints to the `web-ui`, combining storage-backed dashboard/read models with `runtime-core`-backed live metrics, scheduler snapshots, simulation status, and builder state.

## Project Structure

```
mempulse/
├── apps/
│   └── web-ui/                    React 19 + Vite dashboard client
│       └── src/
│           ├── main.jsx           Entry point — mounts React app
│           ├── App.jsx            Root component and routing shell
│           ├── styles.css         Global Tailwind base styles
│           ├── features/
│           │   └── dashboard/     Self-contained dashboard feature module
│           │       ├── components/  UI components (radar, opps, tx dialog, perf panel)
│           │       ├── domain/      Pure domain logic (stores, filters, circular buffer)
│           │       ├── hooks/       React hooks (stream, controller, derived state)
│           │       ├── lib/         Utility models (virtualized tables, animations, health)
│           │       ├── workers/     Web Worker for off-thread SSE stream processing
│           │       └── index.js     Public API of the dashboard feature
│           ├── network/           API base URL resolution and fetch helpers
│           ├── runtime/           Runtime flags, Vite env config, devtools hook
│           ├── shared/            Cross-feature utilities (e.g. Tailwind cn helper)
│           └── test/              Test setup and browser API mocks
├── crates/
│   ├── common/          Shared IDs, alert primitives, metrics helpers, domain types
│   ├── event-log/       Canonical append-only event contracts
│   ├── ingest/          Reusable RPC/devp2p ingest services and tx decode helpers
│   ├── feature-engine/  Protocol classification and MEV feature derivation
│   ├── scheduler/       Pending queue, nonce-gap handling, replacement policy, sim handoff
│   ├── searcher/        Opportunity scoring and explainable ranking output
│   ├── sim-engine/      Deterministic and RPC-backed execution simulation
│   ├── builder/         Candidate assembly and relay dry-run packaging
│   ├── storage/         In-memory projections, WAL recovery, parquet export, ClickHouse adapters
│   ├── replay/          Lifecycle replay, checkpoint parity, replay diff summaries
│   ├── runtime-core/    Long-lived runtime orchestration and live RPC workers
│   ├── node-runtime/    Thin runtime bootstrap and lifecycle wrapper used by viz-api
│   ├── viz-api/         Axum API, SSE transport, metrics, replay, and dashboard endpoints
│   └── bench/           Perf harnesses for tx pipeline, scheduler, simulation, and storage
├── configs/             Chain configuration (chain_config.json)
├── docker/              Dockerfile and ClickHouse setup
├── docs/                Architecture docs, demo guides, implementation plans, perf baselines
└── scripts/             Demo, verification, and perf regression scripts
```

## Common Use Cases

In practical terms, this repo can be used as the backbone for a mempool intelligence and runtime-analysis stack. It can watch live pending transaction flow, classify and score what is happening, persist the event stream for later replay, and expose that state through APIs and a dashboard so operators or researchers can inspect the system in real time.

- Run a live mempool monitoring service across one or more EVM chains using RPC or devp2p ingest.
- Investigate incidents such as dropped transactions, replay mismatches, reorg effects, queue saturation, or latency regressions using persisted events, replay, metrics, and alerts.
- Build internal dashboards for pending transaction flow, derived protocol features, opportunity scoring, relay dry-run state, and per-chain ingest health.
- Prototype and evaluate searcher, simulation, scheduler, and builder logic before integrating those components into a larger production trading or infra stack.

What it is not:

- This repo is not a turnkey profit-generating bot by itself; it is the infrastructure layer for observing, testing, and operating mempool-aware systems.

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

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
