# web-ui

Magic UI-style 2D frontend for mempool replay and transaction analysis.

## Run locally

Start API:

```bash
cargo run -p viz-api --bin viz-api
```

In a second terminal, install dependencies and start the web UI:

```bash
npm install
npm run dev
```

Then open in browser:

- `http://localhost:5174/`

The page calls:

- `GET /replay`
- `GET /propagation`
- `GET /features`
- `GET /features/recent?limit=500`
- `GET /transactions?limit=20`
- `GET /transactions/all?limit=5000`

and renders:

- a SaaS-style 2D split layout (sidebar + list + detail pane)
- animated metric cards and counters (Magic UI-inspired components)
- replay timeline slider bound to pending-count frames
- searchable transaction list with protocol/category tags
- detail pane for transaction metadata + feature-engine analysis

## UI tuning config (recommended)

Use a two-layer config model for memory/perf tuning:

1. Runtime overrides in `runtime-config.js` (preferred for production changes without rebuilds)
2. Build-time defaults from `VITE_*` env vars

Precedence is: runtime config > env config > built-in defaults.

### Runtime config (no rebuild)

Edit `apps/web-ui/public/runtime-config.js` (or replace the deployed `/runtime-config.js` file):

```js
window.__MEMPULSE_UI_CONFIG__ = {
  txSnapshotLimit: 160,
  txHistoryLimit: 600,
  featureHistoryLimit: 1500,
  opportunityHistoryLimit: 1500,
  txRenderLimit: 80,
  streamBatchMs: 1000,
  streamTransport: 'sse', // 'sse' (default) or 'ws'
  workerEnabled: true,
  virtualizedTickerEnabled: true,
  samplingLagThresholdMs: 30,
  samplingStride: 5,
  samplingFlushIdleMs: 500,
  heapEmergencyPurgeMb: 400,
  txRetentionMs: 240000,
  detailCacheLimit: 128,
};
```

Recommended in production:
- Serve `/runtime-config.js` with short/no-cache headers.
- Keep values numeric only.

### Env defaults (requires rebuild/restart)

Create `apps/web-ui/.env.local` (or set env vars in CI/CD):

```bash
VITE_UI_TX_SNAPSHOT_LIMIT=120
VITE_UI_TX_HISTORY_LIMIT=300
VITE_UI_FEATURE_HISTORY_LIMIT=1500
VITE_UI_OPPORTUNITY_HISTORY_LIMIT=1500
VITE_UI_TX_RENDER_LIMIT=80
VITE_UI_STREAM_BATCH_MS=1000
VITE_UI_STREAM_TRANSPORT=sse
VITE_UI_WORKER_ENABLED=true
VITE_UI_VIRTUALIZED_TICKER_ENABLED=true
VITE_UI_SAMPLING_LAG_THRESHOLD_MS=30
VITE_UI_SAMPLING_STRIDE=5
VITE_UI_SAMPLING_FLUSH_IDLE_MS=500
VITE_UI_HEAP_EMERGENCY_PURGE_MB=400
VITE_UI_TX_RETENTION_MS=300000
VITE_UI_DETAIL_CACHE_LIMIT=96
```

Notes:
- These are build-time frontend variables (`VITE_*`), so changing them requires rebuilding/restarting the UI.
- They are not secrets; never put credentials in frontend env vars.

### Stream transport rollout and rollback

Dashboard live stream transport now supports:
- `streamTransport: 'sse'` as the default rollout mode.
- `streamTransport: 'ws'` as the rollback mode.

Runtime toggle example (`apps/web-ui/public/runtime-config.js`):

```js
window.__MEMPULSE_UI_CONFIG__ = {
  streamTransport: 'sse', // default rollout
  // streamTransport: 'ws', // rollback
};
```

Build-time toggle example (`apps/web-ui/.env.local`):

```bash
VITE_UI_STREAM_TRANSPORT=sse
# VITE_UI_STREAM_TRANSPORT=ws
```

## UI performance profiling

The dashboard exposes a lightweight debug surface at runtime:

- `window.__MEMPULSE_PERF__.snapshot()`

This reports:
- long-task summaries for snapshot apply and transaction commit paths
- dropped-frame ratio + rolling FPS estimate
- heap usage aggregates (when `performance.memory` is available)

Quick profiling workflow:

1. Start API and UI (`cargo run -p viz-api --bin viz-api` + `npm run dev`).
2. Open Chrome DevTools Performance panel and record during active stream load.
3. In console, sample `window.__MEMPULSE_PERF__.snapshot()` every few seconds.
4. Check for:
   - repeated long-task spikes (`>=50ms`)
   - growing dropped-frame ratio
   - steady heap growth without plateau

## Stress gate script

Run a local verification pass and emit a timestamped gate summary artifact:

```bash
bash scripts/ui_perf_stress.sh
```

Optional: evaluate gates against a saved `window.__MEMPULSE_PERF__.snapshot()` payload:

```bash
bash scripts/ui_perf_stress.sh --perf-json /path/to/perf-snapshot.json
```

Outputs are written to `artifacts/ui-perf/`.

Notes:
- Sampling mode activates when observed frame delta exceeds configured lag threshold.
- While sampling mode is active, a trailing flush timeout forces buffered rows to render after idle (`samplingFlushIdleMs`).
- If Chrome heap usage exceeds `heapEmergencyPurgeMb`, the dashboard purges live buffers and triggers snapshot resync.

## One-Command Demo

From repository root:

```bash
bash scripts/demo_v2.sh --run
```

Verify endpoint readiness/output contract:

```bash
bash scripts/demo_v2.sh --verify
```

## Windows + WSL note

When the UI is opened from Windows browser and API runs in WSL, pass API base explicitly:

- `http://127.0.0.1:5174/?apiBase=http://<WSL_IP>:3000`

Example:

- `http://127.0.0.1:5174/?apiBase=http://172.20.48.1:3000`

The `apiBase` value is persisted in browser local storage.
