#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "[commercial] verifying explainable searcher scoring"
cargo test -p searcher --test explainable_scoring -- --nocapture

echo "[commercial] verifying rpc-backed simulation mode"
cargo test -p sim-engine --test rpc_backed_mode -- --nocapture

echo "[commercial] verifying storage durability/export checks"
cargo test -p storage --test wal_recovery -- --nocapture
cargo test -p storage --test parquet_export -- --nocapture

echo "[commercial] verifying viz-api auth/quota protections"
cargo test -p viz-api --test api_auth_and_limits -- --nocapture

echo "[commercial] readiness checks: PASS"
