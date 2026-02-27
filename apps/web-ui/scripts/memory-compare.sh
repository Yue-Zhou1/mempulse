#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB_UI_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
HOST="${HOST:-0.0.0.0}"
PORT="${PORT:-5174}"
MODE="${1:-all}"
ACTIVE_PID=""

kill_process_tree() {
  local pid="$1"
  local child_pid

  while IFS= read -r child_pid; do
    [[ -n "${child_pid}" ]] || continue
    kill_process_tree "${child_pid}"
  done < <(pgrep -P "${pid}" 2>/dev/null || true)

  kill "${pid}" >/dev/null 2>&1 || true
}

cleanup_active_pid() {
  if [[ -n "${ACTIVE_PID}" ]]; then
    kill_process_tree "${ACTIVE_PID}"
    if kill -0 "${ACTIVE_PID}" >/dev/null 2>&1; then
      kill -9 "${ACTIVE_PID}" >/dev/null 2>&1 || true
    fi
    wait "${ACTIVE_PID}" >/dev/null 2>&1 || true
    ACTIVE_PID=""
    sleep 1
  fi
}

trap cleanup_active_pid EXIT INT TERM

run_mode() {
  local label="$1"
  shift

  echo
  echo "============================================================"
  echo "${label}"
  echo "URL: http://127.0.0.1:${PORT}"
  echo "建议在 Chrome Task Manager 记录 Memory footprint / JS memory / GPU memory。"
  echo "============================================================"

  (
    cd "${WEB_UI_DIR}"
    "$@"
  ) &
  ACTIVE_PID="$!"

  sleep 2
  read -r -p "观测完成后按 Enter 继续..." _
  cleanup_active_pid
}

run_dev_strict_on() {
  run_mode \
    "DEV: strict=ON, react-devtools-hook=ON, perf-entry-cleanup=ON" \
    env \
      VITE_UI_STRICT_MODE=true \
      VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED=true \
      VITE_UI_DEV_PERF_ENTRY_CLEANUP=true \
      VITE_UI_DEV_PERF_ENTRY_CLEANUP_INTERVAL_MS=15000 \
      npm run dev -- --host "${HOST}" --port "${PORT}"
}

run_dev_strict_off() {
  run_mode \
    "DEV: strict=OFF, react-devtools-hook=ON, perf-entry-cleanup=ON" \
    env \
      VITE_UI_STRICT_MODE=false \
      VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED=true \
      VITE_UI_DEV_PERF_ENTRY_CLEANUP=true \
      VITE_UI_DEV_PERF_ENTRY_CLEANUP_INTERVAL_MS=15000 \
      npm run dev -- --host "${HOST}" --port "${PORT}"
}

run_dev_devtools_off() {
  run_mode \
    "DEV: strict=ON, react-devtools-hook=OFF, perf-entry-cleanup=ON" \
    env \
      VITE_UI_STRICT_MODE=true \
      VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED=false \
      VITE_UI_DEV_PERF_ENTRY_CLEANUP=true \
      VITE_UI_DEV_PERF_ENTRY_CLEANUP_INTERVAL_MS=15000 \
      npm run dev -- --host "${HOST}" --port "${PORT}"
}

run_prod_preview() {
  (
    cd "${WEB_UI_DIR}"
    env \
      VITE_UI_STRICT_MODE=false \
      VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED=false \
      npm run build
  )

  run_mode \
    "PROD PREVIEW: strict=OFF, react-devtools-hook=OFF" \
    env \
      VITE_UI_STRICT_MODE=false \
      VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED=false \
      npm run preview -- --host "${HOST}" --port "${PORT}"
}

case "${MODE}" in
  dev-strict-on)
    run_dev_strict_on
    ;;
  dev-strict-off)
    run_dev_strict_off
    ;;
  dev-devtools-off)
    run_dev_devtools_off
    ;;
  prod)
    run_prod_preview
    ;;
  all)
    run_dev_strict_on
    run_dev_strict_off
    run_dev_devtools_off
    run_prod_preview
    ;;
  *)
    echo "Usage: $0 [dev-strict-on|dev-strict-off|dev-devtools-off|prod|all]" >&2
    exit 1
    ;;
esac
