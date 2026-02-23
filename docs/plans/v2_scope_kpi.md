# V2 Scope and KPI Contract

## Objective

Define measurable, demo-grade acceptance targets for the V2 mempool/MEV infrastructure effort.

## Scope

### In Scope (V2)

- Production-oriented devp2p-first ingest path with RPC fallback
- Deterministic event replay with parity checkpoints
- EVM-aware simulation path for candidate evaluation
- MEV opportunity ranking pipeline
- PBS relay integration in dry-run mode
- Performance profiling and budget enforcement
- Operational observability and alerting
- Reproducible one-command demo workflow

### Out of Scope (V2)

- Live capital deployment
- Production relay bidding with signing keys
- Strategy-specific alpha disclosures
- Cross-chain strategy execution

## KPI Contract

| KPI | Target | Measurement |
|---|---|---|
| Runtime stability | 60+ minutes continuous ingest without OOM or fatal crash | Long-run soak test log and process health |
| Ingest throughput | Sustain >= 300 tx/s in replay benchmark mode | `scripts/profile_pipeline.sh` benchmark report |
| End-to-end latency | p95 <= 200 ms for ingest->decode->feature pipeline | Stage timing metrics and benchmark output |
| Replay determinism | >= 99.99% checkpoint parity | `replay` parity report on captured windows |
| Coverage quality | devp2p ingest coverage >= RPC baseline in same interval | Comparative ingest summary report |
| Searcher budget | Candidate generation+ranking p95 <= 2 s per block window | Searcher benchmark output |
| API reliability | No failed health checks across 60-minute demo | Demo verifier logs |
| Observability completeness | Required alerts and dashboard panels present | `ops/` config validation and screenshots |
| Commercial API controls | Auth-required routes enforce `401`/`429` behavior under invalid/over-limit access | `cargo test -p viz-api --test api_auth_and_limits` |
| Durability and export | WAL recovery + replay export adapter pass recovery/roundtrip tests | `cargo test -p storage --test wal_recovery` + `cargo test -p storage --test parquet_export` |

## Milestones

1. M1: Scope and baseline verification gates updated.
2. M2: devp2p ingest lane and source mode switch.
3. M3: canonical schema expansion and deterministic simulation.
4. M4: opportunity engine and relay dry-run path.
5. M5: performance hardening, observability, storage backfill.
6. M6: demo demo package and final verification.

## Exit Criteria

V2 is considered complete when:

- all KPI targets above are measured and documented,
- `./scripts/verify_phase_checks.sh` passes,
- `bash scripts/verify_commercial_readiness.sh` passes,
- benchmark and demo scripts pass in a clean environment.
