#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

mkdir -p artifacts/replay artifacts/searcher artifacts/builder

echo "[artifacts] generating deterministic replay parity report"
cargo run -p replay --example parity_report -- --out artifacts/replay/parity_report.json

echo "[artifacts] generating searcher backtest summary"
cargo run -p searcher --example backtest_summary -- --out artifacts/searcher/backtest_summary.json

echo "[artifacts] generating relay dry-run trace package"
cargo run -p builder --example relay_trace_package -- --out-dir artifacts/builder

echo "[artifacts] done"
