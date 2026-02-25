#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ARTIFACT_DIR="${TX_PIPELINE_PERF_ARTIFACT_DIR:-artifacts/perf}"
LATEST_ARTIFACT_PATH="${ARTIFACT_DIR}/tx_pipeline_perf_baseline_latest.json"
MAX_SNAPSHOT_LATENCY_MS="${PERF_BUDGET_MAX_SNAPSHOT_LATENCY_MS:-7000}"
MIN_STREAM_EVENTS_PER_SEC="${PERF_BUDGET_MIN_STREAM_EVENTS_PER_SEC:-50000}"
MAX_DROP_RATIO="${PERF_BUDGET_MAX_DROP_RATIO:-0.005}"

echo "perf-regression: running tx pipeline baseline harness"
bash scripts/perf_tx_pipeline_baseline.sh

if [[ ! -f "$LATEST_ARTIFACT_PATH" ]]; then
  echo "perf-regression: missing artifact ${LATEST_ARTIFACT_PATH}" >&2
  exit 1
fi

python3 - "$LATEST_ARTIFACT_PATH" "$MAX_SNAPSHOT_LATENCY_MS" "$MIN_STREAM_EVENTS_PER_SEC" "$MAX_DROP_RATIO" <<'PY'
import json
import sys

artifact_path = sys.argv[1]
max_snapshot_latency_ms = float(sys.argv[2])
min_stream_events_per_sec = float(sys.argv[3])
max_drop_ratio = float(sys.argv[4])

with open(artifact_path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

snapshot_latency_ms = float(payload.get("snapshot_latency_ms", 0.0))
stream_events_per_sec = float(payload.get("stream_events_per_sec", 0.0))
seeded_events = float(payload.get("seeded_events", 0.0))
stream_events_seen = float(payload.get("stream_events_seen", 0.0))

drop_ratio = 0.0
if seeded_events > 0:
    drop_ratio = max(0.0, 1.0 - (stream_events_seen / seeded_events))

print(f"perf-regression: artifact={artifact_path}")
print(f"perf-regression: snapshot_latency_ms={snapshot_latency_ms:.3f}")
print(f"perf-regression: stream_events_per_sec={stream_events_per_sec:.3f}")
print(f"perf-regression: drop_ratio={drop_ratio:.6f}")

violations = []
if snapshot_latency_ms > max_snapshot_latency_ms:
    violations.append(
        f"snapshot_latency_ms {snapshot_latency_ms:.3f} exceeds budget {max_snapshot_latency_ms:.3f}"
    )
if stream_events_per_sec < min_stream_events_per_sec:
    violations.append(
        f"stream_events_per_sec {stream_events_per_sec:.3f} below budget {min_stream_events_per_sec:.3f}"
    )
if drop_ratio > max_drop_ratio:
    violations.append(
        f"drop_ratio {drop_ratio:.6f} exceeds budget {max_drop_ratio:.6f}"
    )

if violations:
    for violation in violations:
        print(f"perf-regression: FAIL {violation}", file=sys.stderr)
    sys.exit(1)

print("perf-regression: PASS budgets satisfied")
PY
