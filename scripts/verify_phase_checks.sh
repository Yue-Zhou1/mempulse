#!/usr/bin/env bash
set -euo pipefail

if [[ ! -f "docs/plans/v2_scope_kpi.md" ]]; then
  echo "[verify] missing required file: docs/plans/v2_scope_kpi.md"
  exit 1
fi

echo "[verify] running workspace tests"
cargo test --workspace

echo "[verify] building replay and viz binaries"
cargo build -p replay --bin replay-cli
cargo build -p viz-api --bin viz-api

if [[ "${VERIFY_COMMERCIAL_READINESS:-1}" == "1" ]]; then
  echo "[verify] running commercial readiness gate"
  bash scripts/verify_commercial_readiness.sh
else
  echo "[verify] skipping commercial readiness gate (VERIFY_COMMERCIAL_READINESS=${VERIFY_COMMERCIAL_READINESS:-0})"
fi

echo "[verify] phase acceptance checks: PASS"
