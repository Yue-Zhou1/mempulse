#!/usr/bin/env bash
set -euo pipefail

echo "[verify-static] checking formatting"
cargo fmt --all -- --check

echo "[verify-static] compiling workspace targets"
cargo check --workspace --all-targets

echo "[verify-static] enforcing lint-free workspace"
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "[verify-static] static checks: PASS"
