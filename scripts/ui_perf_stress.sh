#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="$ROOT_DIR/artifacts/ui-perf"
mkdir -p "$ARTIFACT_DIR"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
SUMMARY_PATH="$ARTIFACT_DIR/${STAMP}-gate-summary.md"
PERF_JSON_PATH=""

usage() {
  cat <<'USAGE'
Usage:
  bash scripts/ui_perf_stress.sh [--perf-json <path>]

Options:
  --perf-json <path>  Path to JSON exported from window.__MEMPULSE_PERF__.snapshot().
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --perf-json)
      if [[ $# -lt 2 ]]; then
        echo "--perf-json requires a file path" >&2
        exit 1
      fi
      PERF_JSON_PATH="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

cd "$ROOT_DIR"

echo "[ui-perf] Running web-ui tests..."
npm --prefix apps/web-ui test

echo "[ui-perf] Running web-ui production build..."
npm --prefix apps/web-ui run build

echo "[ui-perf] Writing gate summary to $SUMMARY_PATH"
node - "$SUMMARY_PATH" "$PERF_JSON_PATH" <<'NODE'
const fs = require('node:fs');

const [summaryPath, perfPath] = process.argv.slice(2);

const nowIso = new Date().toISOString();
const lines = [];

function formatBytesToMb(bytes) {
  if (!Number.isFinite(bytes)) {
    return 'n/a';
  }
  return `${Math.round(bytes / (1024 * 1024))} MB`;
}

function readPerfSnapshot(path) {
  if (!path) {
    return { snapshot: null, note: 'No perf snapshot provided. Gate checks remain unverified.' };
  }
  if (!fs.existsSync(path)) {
    return { snapshot: null, note: `Perf snapshot file not found: ${path}` };
  }

  const raw = fs.readFileSync(path, 'utf8');
  const parsed = JSON.parse(raw);
  const snapshot = parsed && parsed.snapshot ? parsed.snapshot : parsed;
  if (!snapshot || typeof snapshot !== 'object') {
    return { snapshot: null, note: `Perf snapshot is invalid JSON shape: ${path}` };
  }
  return { snapshot, note: `Loaded perf snapshot from ${path}` };
}

const { snapshot, note } = readPerfSnapshot(perfPath);
lines.push('# UI Perf Gate Summary');
lines.push('');
lines.push(`- Generated: ${nowIso}`);
lines.push(`- Perf input: ${note}`);
lines.push('');
lines.push('## Verification Commands');
lines.push('');
lines.push('- `npm --prefix apps/web-ui test`');
lines.push('- `npm --prefix apps/web-ui run build`');
lines.push('');
lines.push('## Release Gates');
lines.push('');

const gates = [];

function addGate(name, status, detail) {
  gates.push({ name, status, detail });
}

if (!snapshot) {
  addGate('Heap plateaus under 200 MB', 'UNVERIFIED', 'No perf snapshot JSON provided.');
  addGate('No sustained Long Task over 50 ms', 'UNVERIFIED', 'No perf snapshot JSON provided.');
  addGate('Render loop >= 50 FPS', 'UNVERIFIED', 'No perf snapshot JSON provided.');
  addGate('Emergency heap threshold <= 400 MB', 'UNVERIFIED', 'No perf snapshot JSON provided.');
} else {
  const snapshotApplyMaxMs = Number(snapshot?.snapshotApply?.maxMs ?? 0);
  const commitMaxMs = Number(snapshot?.transactionCommit?.maxMs ?? 0);
  const longTaskMaxMs = Math.max(snapshotApplyMaxMs, commitMaxMs);
  const fps = Number(snapshot?.frame?.rollingFps ?? 0);
  const droppedRatio = Number(snapshot?.frame?.ratio ?? 0);
  const heapLatestBytes = Number(snapshot?.heap?.latestBytes ?? 0);
  const heapPeakBytes = Number(snapshot?.heap?.peakBytes ?? 0);

  addGate(
    'Heap plateaus under 200 MB',
    heapLatestBytes <= 200 * 1024 * 1024 ? 'PASS' : 'FAIL',
    `latest=${formatBytesToMb(heapLatestBytes)}, peak=${formatBytesToMb(heapPeakBytes)}`,
  );
  addGate(
    'No sustained Long Task over 50 ms',
    longTaskMaxMs <= 50 ? 'PASS' : 'FAIL',
    `maxLongTask=${Math.round(longTaskMaxMs)} ms`,
  );
  addGate(
    'Render loop >= 50 FPS',
    fps >= 50 ? 'PASS' : 'FAIL',
    `rollingFps=${fps.toFixed(1)}, droppedFrameRatio=${(droppedRatio * 100).toFixed(1)}%`,
  );
  addGate(
    'Emergency heap threshold <= 400 MB',
    heapLatestBytes <= 400 * 1024 * 1024 ? 'PASS' : 'FAIL',
    `latest=${formatBytesToMb(heapLatestBytes)}`,
  );
}

for (const gate of gates) {
  lines.push(`- [${gate.status}] ${gate.name}: ${gate.detail}`);
}

lines.push('');
lines.push('## Notes');
lines.push('');
lines.push('- Interaction latency p95 and 30-minute steady-state validation still require manual browser profiling.');
lines.push('- Use Chrome DevTools + `window.__MEMPULSE_PERF__.snapshot()` during synthetic stream load for final sign-off.');

fs.writeFileSync(summaryPath, `${lines.join('\n')}\n`);

if (snapshot) {
  const hasFailure = gates.some((gate) => gate.status === 'FAIL');
  if (hasFailure) {
    process.exitCode = 2;
  }
}
NODE

echo "[ui-perf] Summary written: $SUMMARY_PATH"
