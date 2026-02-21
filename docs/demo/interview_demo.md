# V2 Interview Demo Workflow

## One-Command Verify

Run:

```bash
bash scripts/demo_v2.sh --verify
```

This command starts a local `viz-api` instance and validates endpoint shape/readiness for:

1. `/health`
2. `/replay`
3. `/propagation`
4. `/metrics/snapshot`
5. `/alerts/evaluate`
6. `/relay/dry-run/status`

## One-Command Run

Run:

```bash
bash scripts/demo_v2.sh --run
```

This launches:

1. `viz-api` (default `127.0.0.1:3100`)
2. web UI static server (default `127.0.0.1:5174`)

Open:

- `http://127.0.0.1:5174/?apiBase=http://127.0.0.1:3100`

## Interview Narrative

1. Show ingest/replay API health and replay output shape.
2. Show feature/searcher outputs in the UI transaction list and details.
3. Show alert and metrics endpoints (`/metrics/snapshot`, `/alerts/evaluate`).
4. Show relay dry-run status endpoint (`/relay/dry-run/status`).
5. Reference `docs/perf/v2_baseline.md` for measured latency budget evidence.
