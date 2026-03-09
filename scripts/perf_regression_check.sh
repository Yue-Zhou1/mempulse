#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ARTIFACT_DIR="${TX_PIPELINE_PERF_ARTIFACT_DIR:-artifacts/perf}"
TX_LATEST_ARTIFACT_PATH="${ARTIFACT_DIR}/tx_pipeline_perf_baseline_latest.json"
SCHEDULER_LATEST_ARTIFACT_PATH="${ARTIFACT_DIR}/scheduler_pipeline_latest.json"
SIMULATION_LATEST_ARTIFACT_PATH="${ARTIFACT_DIR}/simulation_roundtrip_latest.json"
STORAGE_LATEST_ARTIFACT_PATH="${ARTIFACT_DIR}/storage_snapshot_latest.json"
MAX_SNAPSHOT_LATENCY_MS="${PERF_BUDGET_MAX_SNAPSHOT_LATENCY_MS:-7000}"
MIN_STREAM_EVENTS_PER_SEC="${PERF_BUDGET_MIN_STREAM_EVENTS_PER_SEC:-50000}"
MAX_DROP_RATIO="${PERF_BUDGET_MAX_DROP_RATIO:-0.005}"
MAX_SCHEDULER_PIPELINE_P95_US="${PERF_BUDGET_MAX_SCHEDULER_PIPELINE_P95_US:-50000}"
MAX_SIMULATION_ROUNDTRIP_P95_US="${PERF_BUDGET_MAX_SIMULATION_ROUNDTRIP_P95_US:-50000}"
MAX_STORAGE_SNAPSHOT_WRITE_P95_US="${PERF_BUDGET_MAX_STORAGE_SNAPSHOT_WRITE_P95_US:-25000}"
MAX_STORAGE_SNAPSHOT_REHYDRATE_P95_US="${PERF_BUDGET_MAX_STORAGE_SNAPSHOT_REHYDRATE_P95_US:-25000}"

required_artifacts=(
  "${TX_LATEST_ARTIFACT_PATH}"
  "${SCHEDULER_LATEST_ARTIFACT_PATH}"
  "${SIMULATION_LATEST_ARTIFACT_PATH}"
  "${STORAGE_LATEST_ARTIFACT_PATH}"
)

echo "perf-regression: running tx pipeline baseline harness"
bash scripts/perf_tx_pipeline_baseline.sh

echo "perf-regression: running scheduler pipeline harness"
BENCH_SCHEDULER_PIPELINE_PERF_ARTIFACT="${SCHEDULER_LATEST_ARTIFACT_PATH}" \
  cargo test -p bench --test scheduler_pipeline_perf -- --nocapture

echo "perf-regression: running simulation roundtrip harness"
BENCH_SIMULATION_ROUNDTRIP_PERF_ARTIFACT="${SIMULATION_LATEST_ARTIFACT_PATH}" \
  cargo test -p bench --test simulation_roundtrip_perf -- --nocapture

echo "perf-regression: running storage snapshot harness"
BENCH_STORAGE_SNAPSHOT_PERF_ARTIFACT="${STORAGE_LATEST_ARTIFACT_PATH}" \
  cargo test -p bench --test storage_snapshot_perf -- --nocapture

for artifact_path in "${required_artifacts[@]}"; do
  if [[ ! -f "${artifact_path}" ]]; then
    echo "perf-regression: missing artifact ${artifact_path}" >&2
    exit 1
  fi
done

python3 - \
  "$TX_LATEST_ARTIFACT_PATH" \
  "$SCHEDULER_LATEST_ARTIFACT_PATH" \
  "$SIMULATION_LATEST_ARTIFACT_PATH" \
  "$STORAGE_LATEST_ARTIFACT_PATH" \
  "$MAX_SNAPSHOT_LATENCY_MS" \
  "$MIN_STREAM_EVENTS_PER_SEC" \
  "$MAX_DROP_RATIO" \
  "$MAX_SCHEDULER_PIPELINE_P95_US" \
  "$MAX_SIMULATION_ROUNDTRIP_P95_US" \
  "$MAX_STORAGE_SNAPSHOT_WRITE_P95_US" \
  "$MAX_STORAGE_SNAPSHOT_REHYDRATE_P95_US" <<'PY'
import json
import sys

tx_artifact_path = sys.argv[1]
scheduler_artifact_path = sys.argv[2]
simulation_artifact_path = sys.argv[3]
storage_artifact_path = sys.argv[4]
max_snapshot_latency_ms = float(sys.argv[5])
min_stream_events_per_sec = float(sys.argv[6])
max_drop_ratio = float(sys.argv[7])
max_scheduler_pipeline_p95_us = float(sys.argv[8])
max_simulation_roundtrip_p95_us = float(sys.argv[9])
max_storage_snapshot_write_p95_us = float(sys.argv[10])
max_storage_snapshot_rehydrate_p95_us = float(sys.argv[11])

with open(tx_artifact_path, "r", encoding="utf-8") as handle:
    tx_payload = json.load(handle)

with open(scheduler_artifact_path, "r", encoding="utf-8") as handle:
    scheduler_payload = json.load(handle)

with open(simulation_artifact_path, "r", encoding="utf-8") as handle:
    simulation_payload = json.load(handle)

with open(storage_artifact_path, "r", encoding="utf-8") as handle:
    storage_payload = json.load(handle)

snapshot_latency_ms = float(tx_payload.get("snapshot_latency_ms", 0.0))
stream_events_per_sec = float(tx_payload.get("stream_events_per_sec", 0.0))
seeded_events = float(tx_payload.get("seeded_events", 0.0))
stream_events_seen = float(tx_payload.get("stream_events_seen", 0.0))
scheduler_pipeline_p95_us = float(scheduler_payload.get("p95_us", 0.0))
simulation_roundtrip_p95_us = float(simulation_payload.get("p95_us", 0.0))
storage_snapshot_write_p95_us = float(storage_payload.get("write_p95_us", 0.0))
storage_snapshot_rehydrate_p95_us = float(storage_payload.get("rehydrate_p95_us", 0.0))

drop_ratio = 0.0
if seeded_events > 0:
    drop_ratio = max(0.0, 1.0 - (stream_events_seen / seeded_events))

print(f"perf-regression: tx_artifact={tx_artifact_path}")
print(f"perf-regression: snapshot_latency_ms={snapshot_latency_ms:.3f}")
print(f"perf-regression: stream_events_per_sec={stream_events_per_sec:.3f}")
print(f"perf-regression: drop_ratio={drop_ratio:.6f}")
print(f"perf-regression: scheduler_pipeline_p95_us={scheduler_pipeline_p95_us:.3f}")
print(f"perf-regression: simulation_roundtrip_p95_us={simulation_roundtrip_p95_us:.3f}")
print(f"perf-regression: storage_snapshot_write_p95_us={storage_snapshot_write_p95_us:.3f}")
print(f"perf-regression: storage_snapshot_rehydrate_p95_us={storage_snapshot_rehydrate_p95_us:.3f}")

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
if scheduler_pipeline_p95_us > max_scheduler_pipeline_p95_us:
    violations.append(
        "scheduler_pipeline_p95_us "
        f"{scheduler_pipeline_p95_us:.3f} exceeds budget {max_scheduler_pipeline_p95_us:.3f}"
    )
if simulation_roundtrip_p95_us > max_simulation_roundtrip_p95_us:
    violations.append(
        "simulation_roundtrip_p95_us "
        f"{simulation_roundtrip_p95_us:.3f} exceeds budget {max_simulation_roundtrip_p95_us:.3f}"
    )
if storage_snapshot_write_p95_us > max_storage_snapshot_write_p95_us:
    violations.append(
        "storage_snapshot_write_p95_us "
        f"{storage_snapshot_write_p95_us:.3f} exceeds budget {max_storage_snapshot_write_p95_us:.3f}"
    )
if storage_snapshot_rehydrate_p95_us > max_storage_snapshot_rehydrate_p95_us:
    violations.append(
        "storage_snapshot_rehydrate_p95_us "
        f"{storage_snapshot_rehydrate_p95_us:.3f} exceeds budget {max_storage_snapshot_rehydrate_p95_us:.3f}"
    )

if violations:
    for violation in violations:
        print(f"perf-regression: FAIL {violation}", file=sys.stderr)
    sys.exit(1)

print("perf-regression: PASS budgets satisfied")
PY
