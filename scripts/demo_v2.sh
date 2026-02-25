#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
if [[ "$MODE" != "--verify" && "$MODE" != "--run" ]]; then
  echo "usage: $0 --verify | --run"
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

API_BIND_HOST="${DEMO_V2_API_BIND_HOST:-0.0.0.0}"
API_PUBLIC_HOST="${DEMO_V2_API_PUBLIC_HOST:-127.0.0.1}"
API_PORT="${DEMO_V2_API_PORT:-3100}"
API_BASE="http://${API_PUBLIC_HOST}:${API_PORT}"
UI_BIND_HOST="${DEMO_V2_UI_BIND_HOST:-0.0.0.0}"
UI_PUBLIC_HOST="${DEMO_V2_UI_PUBLIC_HOST:-127.0.0.1}"
UI_PORT="${DEMO_V2_UI_PORT:-5174}"
API_HEALTH_TIMEOUT_SECONDS="${DEMO_V2_API_HEALTH_TIMEOUT_SECONDS:-180}"
API_HEALTH_POLL_INTERVAL_SECONDS=0.5
UI_START_TIMEOUT_SECONDS="${DEMO_V2_UI_START_TIMEOUT_SECONDS:-60}"
LOG_DIR="target/demo"
mkdir -p "$LOG_DIR"

API_LOCAL_HOST="${API_BIND_HOST}"
if [[ "$API_LOCAL_HOST" == "0.0.0.0" ]]; then
  API_LOCAL_HOST="127.0.0.1"
fi
API_LOCAL_BASE="http://${API_LOCAL_HOST}:${API_PORT}"

UI_LOCAL_HOST="${UI_BIND_HOST}"
if [[ "$UI_LOCAL_HOST" == "0.0.0.0" ]]; then
  UI_LOCAL_HOST="127.0.0.1"
fi
UI_LOCAL_BASE="http://${UI_LOCAL_HOST}:${UI_PORT}"

WSL_EXTERNAL_HOST=""
if grep -qi microsoft /proc/version 2>/dev/null; then
  WSL_EXTERNAL_HOST="$(hostname -I 2>/dev/null | awk '{print $1}')"
fi

API_LOG="$LOG_DIR/viz-api.log"
UI_LOG="$LOG_DIR/web-ui.log"

api_pid=""
ui_pid=""

cleanup() {
  if [[ -n "${ui_pid}" ]] && kill -0 "${ui_pid}" 2>/dev/null; then
    kill "${ui_pid}" 2>/dev/null || true
  fi
  if [[ -n "${api_pid}" ]] && kill -0 "${api_pid}" 2>/dev/null; then
    kill "${api_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

start_api() {
  RUST_LOG="${RUST_LOG:-warn}" \
  VIZ_API_BIND="${API_BIND_HOST}:${API_PORT}" \
  VIZ_API_INGEST_MODE="${VIZ_API_INGEST_MODE:-rpc}" \
    cargo run -p viz-api --bin viz-api >"$API_LOG" 2>&1 &
  api_pid="$!"
}

wait_for_health() {
  local polls
  polls="$(python3 - <<PY
timeout_seconds = float("${API_HEALTH_TIMEOUT_SECONDS}")
poll_interval = float("${API_HEALTH_POLL_INTERVAL_SECONDS}")
print(max(1, int(timeout_seconds / poll_interval)))
PY
)"

  for _ in $(seq 1 "$polls"); do
    if curl -fsS "${API_LOCAL_BASE}/health" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "${api_pid}" ]] && ! kill -0 "${api_pid}" 2>/dev/null; then
      return 1
    fi
    sleep "${API_HEALTH_POLL_INTERVAL_SECONDS}"
  done
  return 1
}

verify_endpoint_shapes() {
  curl -fsS "${API_LOCAL_BASE}/health" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("status")=="ok"'
  curl -fsS "${API_LOCAL_BASE}/replay" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert isinstance(d,list)'
  curl -fsS "${API_LOCAL_BASE}/propagation" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert isinstance(d,list); assert len(d)>=1; assert "source" in d[0] and "destination" in d[0]'
  curl -fsS "${API_LOCAL_BASE}/metrics/snapshot" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "ingest_lag_ms" in d and "queue_depth_current" in d'
  curl -fsS "${API_LOCAL_BASE}/alerts/evaluate" | python3 -c 'import json,sys; d=json.load(sys.stdin); required={"ingest_lag","decode_failure","coverage_collapse","queue_saturation"}; assert required.issubset(d.keys())'
  curl -fsS "${API_LOCAL_BASE}/relay/dry-run/status" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "total_submissions" in d and "latest" in d'
}

start_ui() {
  (
    cd apps/web-ui
    WEB_UI_API_PROXY_TARGET="${API_LOCAL_BASE}" \
      npm run dev -- --host "${UI_BIND_HOST}" --port "${UI_PORT}"
  ) >"$UI_LOG" 2>&1 &
  ui_pid="$!"
}

wait_for_ui() {
  local polls
  polls="$(python3 - <<PY
timeout_seconds = float("${UI_START_TIMEOUT_SECONDS}")
poll_interval = float("${API_HEALTH_POLL_INTERVAL_SECONDS}")
print(max(1, int(timeout_seconds / poll_interval)))
PY
)"

  for _ in $(seq 1 "$polls"); do
    if curl -fsS "${UI_LOCAL_BASE}/" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "${ui_pid}" ]] && ! kill -0 "${ui_pid}" 2>/dev/null; then
      return 1
    fi
    sleep "${API_HEALTH_POLL_INTERVAL_SECONDS}"
  done
  return 1
}

if [[ "$MODE" == "--verify" ]]; then
  start_api
  if ! wait_for_health; then
    echo "demo-v2 verify: FAIL (api health did not become ready)"
    echo "see log: $API_LOG"
    exit 1
  fi
  verify_endpoint_shapes
  echo "demo-v2 verify: PASS"
  echo "api_base=${API_BASE}"
  exit 0
fi

start_api
if ! wait_for_health; then
  echo "demo-v2 run: FAIL (api health did not become ready)"
  echo "see log: $API_LOG"
  exit 1
fi

start_ui
if ! wait_for_ui; then
  echo "demo-v2 run: FAIL (web-ui failed to start)"
  echo "see log: $UI_LOG"
  exit 1
fi

cat <<EOF2
demo-v2 run: PASS
API: ${API_BASE}
UI : http://${UI_PUBLIC_HOST}:${UI_PORT}/?apiBase=%2Fapi
$(if [[ -n "${WSL_EXTERNAL_HOST}" ]]; then echo "UI (WSL): http://${WSL_EXTERNAL_HOST}:${UI_PORT}/?apiBase=%2Fapi"; fi)

Press Ctrl+C to stop.
EOF2

wait
