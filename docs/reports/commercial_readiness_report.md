# Commercial Readiness Verification Report

## Scope

This report defines the commercial-readiness verification gate that complements the V2 demo checks.

## Verification Command

```bash
bash scripts/verify_commercial_readiness.sh
```

## Required Checks

1. Explainable ranking outputs (`searcher`)
   - `cargo test -p searcher --test explainable_scoring -- --nocapture`
2. RPC-backed simulation mode (`sim-engine`)
   - `cargo test -p sim-engine --test rpc_backed_mode -- --nocapture`
3. Durable storage + export (`storage`)
   - `cargo test -p storage --test wal_recovery -- --nocapture`
   - `cargo test -p storage --test parquet_export -- --nocapture`
4. Protected API behavior (`viz-api`)
   - `cargo test -p viz-api --test api_auth_and_limits -- --nocapture`

## Pass Criteria

- All commands above pass in a clean environment.
- `scripts/verify_phase_checks.sh` includes this gate (default enabled).
- Any failures are treated as release blockers for commercial-facing builds.
