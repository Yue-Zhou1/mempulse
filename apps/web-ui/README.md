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
  txRenderLimit: 80,
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
VITE_UI_TX_RENDER_LIMIT=80
VITE_UI_TX_RETENTION_MS=300000
VITE_UI_DETAIL_CACHE_LIMIT=96
```

Notes:
- These are build-time frontend variables (`VITE_*`), so changing them requires rebuilding/restarting the UI.
- They are not secrets; never put credentials in frontend env vars.

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
