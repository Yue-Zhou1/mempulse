# Relay Dry-Run Trace Package

## Scope

This report packages relay submission dry-run traces, including success-after-retry and terminal failure scenarios, using the `builder` crate.

## Reproduce

```bash
bash scripts/generate_artifacts.sh
cat artifacts/builder/relay_trace_summary.json
cat artifacts/builder/relay_trace_flaky_success.json
cat artifacts/builder/relay_trace_exhausted_failure.json
```

## Scenario Summary

From `artifacts/builder/relay_trace_summary.json`:

1. `flaky_success`
- accepted: `true`
- final_state: `accepted`
- attempts: `2`
- includes HTTP 5xx: `true`

2. `exhausted_failure`
- accepted: `false`
- final_state: `exhausted`
- attempts: `3`
- includes transport errors: `true`

## Failure-Handling Evidence

- Retry/backoff behavior is explicit per attempt in `RelayAttemptTrace.backoff_ms`.
- Flaky scenario demonstrates recovery after `503` on first attempt.
- Exhausted scenario demonstrates bounded retries with terminal failure state.

## Artifacts

- `artifacts/builder/relay_trace_summary.json`
- `artifacts/builder/relay_trace_flaky_success.json`
- `artifacts/builder/relay_trace_exhausted_failure.json`
