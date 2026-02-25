#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ARTIFACT_DIR="${TX_PIPELINE_PERF_ARTIFACT_DIR:-artifacts/perf}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_PATH="${VIZ_API_TX_PERF_ARTIFACT:-${ARTIFACT_DIR}/tx_pipeline_perf_baseline_${TIMESTAMP}.json}"
LATEST_PATH="${ARTIFACT_DIR}/tx_pipeline_perf_baseline_latest.json"

mkdir -p "$ARTIFACT_DIR"

echo "tx-pipeline-baseline: running perf harness"
echo "tx-pipeline-baseline: artifact path ${ARTIFACT_PATH}"

VIZ_API_TX_PERF_ARTIFACT="$ARTIFACT_PATH" cargo test -p viz-api --test tx_pipeline_perf_baseline -- --nocapture

cp "$ARTIFACT_PATH" "$LATEST_PATH"

echo "tx-pipeline-baseline: wrote ${ARTIFACT_PATH}"
echo "tx-pipeline-baseline: updated ${LATEST_PATH}"
