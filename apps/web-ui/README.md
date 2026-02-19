# web-ui

Custom frontend for mempool replay and propagation visualization.

## Run locally

Start API:

```bash
cargo run -p viz-api --bin viz-api
```

In a second terminal, start web UI static server:

```bash
npm run dev
```

Then open in browser:

- `http://localhost:5173/apps/web-ui/`

The page calls:

- `GET /replay`
- `GET /propagation`
- `GET /features`
- `GET /transactions?limit=20`

and renders:

- 3D propagation graph (nodes + latency-colored edges)
- replay timeline slider bound to pending-count frames
- protocol/category summary list
- latest mempool transaction list (hash/sender/nonce/type)

## Windows + WSL note

When the UI is opened from Windows browser and API runs in WSL, pass API base explicitly:

- `http://127.0.0.1:5173/?apiBase=http://<WSL_IP>:3000`

Example:

- `http://127.0.0.1:5173/?apiBase=http://172.20.48.1:3000`

The `apiBase` value is persisted in browser local storage.
