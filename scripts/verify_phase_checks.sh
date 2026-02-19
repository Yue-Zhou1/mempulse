#!/usr/bin/env bash
set -euo pipefail

echo "[verify] running workspace tests"
cargo test --workspace

echo "[verify] building replay and viz binaries"
cargo build -p replay --bin replay-cli
cargo build -p viz-api --bin viz-api

echo "[verify] phase acceptance checks: PASS"
