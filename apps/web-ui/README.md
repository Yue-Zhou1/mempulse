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

and renders:

- 3D propagation graph (nodes + latency-colored edges)
- replay timeline slider bound to pending-count frames
- protocol/category summary list
