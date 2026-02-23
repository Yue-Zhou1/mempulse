# MEV V2 Incident Response Runbook

## Scope

This runbook covers ingest/decoding/coverage incidents surfaced by the V2 alert pack.

## Alert Classes

1. `MempoolQueueSaturation` (critical)
2. `MempoolIngestLagHigh` (warning)
3. `DecodeFailureRateHigh` (warning)
4. `CoverageCollapse` (warning)

## Initial Triage (first 5 minutes)

1. Confirm active alerts in Prometheus and Grafana (`ops/grafana/dashboards/v2_overview.json`).
2. Query API snapshots:
   - `GET /metrics/snapshot`
   - `GET /alerts/evaluate`
3. Capture current deploy + runtime state:
   - git SHA
   - `VIZ_API_INGEST_MODE`
   - `VIZ_API_AUTH_ENABLED` + key rotation status
   - upstream RPC/WS endpoint status

## Playbooks

### Queue Saturation

1. Check ingest source mode and event burst rates.
2. Reduce pressure:
   - temporarily switch to `rpc` mode if `hybrid`/`p2p` is unstable.
   - lower worker fan-in or drop low-value tx categories.
3. Validate queue recovery:
   - `queue_depth_current / queue_depth_capacity` below 0.7 for 10 minutes.

### Ingest Lag

1. Inspect upstream WS health and reconnect churn.
2. Verify local resource pressure (CPU throttling, memory contention).
3. Restart ingest component only after collecting baseline metrics.

### Decode Failure Rate

1. Sample failing tx hashes from ingest logs.
2. Confirm schema parity and tx-type handling changes.
3. Roll forward decoder patch or roll back incompatible deploy.

### Coverage Collapse

1. Compare current tx/sec against baseline and upstream mempool view.
2. Check for RPC provider degradation or regional network partition.
3. Switch to fallback endpoint and track ratio recovery over 2 blocks.

### API Auth / Quota Errors

1. Verify caller key is present in `VIZ_API_API_KEYS`.
2. Check rate-limit budget (`VIZ_API_RATE_LIMIT_PER_MINUTE`) against observed request volume.
3. Confirm failures are `401` (auth) vs `429` (quota) and communicate mitigation.

## Exit Criteria

Incident is resolved when:

1. All active alerts are clear for 15 minutes.
2. `GET /alerts/evaluate` returns no active warning/critical flags.
3. Postmortem notes include timeline, root cause, and prevention action items.
