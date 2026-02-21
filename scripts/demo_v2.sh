#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
if [[ "$MODE" != "--verify" && "$MODE" != "--run" ]]; then
  echo "usage: $0 --verify | --run"
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

API_HOST="${DEMO_V2_API_HOST:-127.0.0.1}"
API_PORT="${DEMO_V2_API_PORT:-3100}"
API_BASE="http://${API_HOST}:${API_PORT}"
UI_PORT="${DEMO_V2_UI_PORT:-5174}"
LOG_DIR="target/demo"
mkdir -p "$LOG_DIR"

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
  VIZ_API_BIND="${API_HOST}:${API_PORT}" \
  VIZ_API_INGEST_MODE="${VIZ_API_INGEST_MODE:-p2p}" \
    cargo run -p viz-api --bin viz-api >"$API_LOG" 2>&1 &
  api_pid="$!"
}

wait_for_health() {
  local deadline=40
  for _ in $(seq 1 "$deadline"); do
    if curl -fsS "${API_BASE}/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

verify_endpoint_shapes() {
  curl -fsS "${API_BASE}/health" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("status")=="ok"'
  curl -fsS "${API_BASE}/replay" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert isinstance(d,list)'
  curl -fsS "${API_BASE}/propagation" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert isinstance(d,list); assert len(d)>=1; assert "source" in d[0] and "destination" in d[0]'
  curl -fsS "${API_BASE}/metrics/snapshot" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "ingest_lag_ms" in d and "queue_depth_current" in d'
  curl -fsS "${API_BASE}/alerts/evaluate" | python3 -c 'import json,sys; d=json.load(sys.stdin); required={"ingest_lag","decode_failure","coverage_collapse","queue_saturation"}; assert required.issubset(d.keys())'
  curl -fsS "${API_BASE}/relay/dry-run/status" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "total_submissions" in d and "latest" in d'
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

(
  cd apps/web-ui
  python3 -m http.server "${UI_PORT}"
) >"$UI_LOG" 2>&1 &
ui_pid="$!"

sleep 0.5
if ! kill -0 "${ui_pid}" 2>/dev/null; then
  echo "demo-v2 run: FAIL (web-ui failed to start)"
  echo "see log: $UI_LOG"
  exit 1
fi

cat <<EOF
demo-v2 run: PASS
API: ${API_BASE}
UI : http://127.0.0.1:${UI_PORT}/?apiBase=${API_BASE}

Press Ctrl+C to stop.
EOF

wait
