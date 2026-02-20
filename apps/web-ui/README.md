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

## Windows + WSL note

When the UI is opened from Windows browser and API runs in WSL, pass API base explicitly:

- `http://127.0.0.1:5174/?apiBase=http://<WSL_IP>:3000`

Example:

- `http://127.0.0.1:5174/?apiBase=http://172.20.48.1:3000`

The `apiBase` value is persisted in browser local storage.
