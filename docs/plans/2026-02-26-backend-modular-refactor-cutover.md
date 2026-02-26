# Backend Modular Runtime Cutover Runbook (2026-02-26)

## Scope

- Promote the backend refactor that introduces `node-runtime` as the runtime composition boundary.
- Keep ingest, storage writer, feature/search/sim, and API serving in one process for low-latency hot paths.
- Keep existing `viz-api` endpoint contracts unchanged.

## Preconditions

1. `bash scripts/verify_phase_checks.sh` passes.
2. `bash scripts/perf_tx_pipeline_baseline.sh` regression is within the accepted budget (target `<= 5%`).
3. `cargo test -p viz-api binary_bootstrap_uses_runtime_builder_contract -- --exact` passes.
4. `cargo test -p viz-api default_state_is_pure_and_does_not_spawn_ingest -- --exact` passes.

## Rollout Plan

1. Deploy to staging with `VIZ_API_INGEST_MODE=rpc` and default `VIZ_API_BIND`.
2. Verify startup logs show `viz-api listening` and `ingest_mode=rpc`.
3. Verify health and API surface:
   - `curl -fsS http://<host>:3000/health`
   - `curl -fsS http://<host>:3000/metrics`
4. Run canary production rollout (10% traffic / one instance) for 30 minutes.
5. If stable, promote to full production rollout.

## Observability Checks During Cutover

- No spike in storage queue drops (`storage_queue_full`, `storage_queue_closed` metrics).
- Stable chain ingest status for configured chains (`/dashboard/snapshot` and chain status routes).
- No increase in replay/API error rate (`5xx`) above baseline.
- End-to-end ingest-to-API latency remains within existing SLO budget.

## Rollback Plan

1. Stop new deployment and redeploy the last known-good backend revision.
2. Keep existing storage/runtime env configuration (including `VIZ_API_INGEST_MODE`) unchanged unless root cause is config drift.
3. Re-run health and metrics checks on rolled-back instances.
4. Preserve logs/metrics from failed cutover window for root-cause analysis.

## Post-Cutover Checklist

1. Record baseline/perf output artifacts for this rollout.
2. Confirm on-call handoff includes the new `node-runtime` lifecycle boundary ownership.
3. Open follow-up issues for any non-blocking warnings observed during rollout.
