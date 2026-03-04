import { useEffect, useMemo, useState } from 'react';

const PERF_POLL_INTERVAL_MS = 1000;

function resolvePerfSnapshot() {
  const perfApi = globalThis?.window?.__MEMPULSE_PERF__;
  if (typeof perfApi?.snapshot !== 'function') {
    return null;
  }
  try {
    return perfApi.snapshot();
  } catch {
    return null;
  }
}

function formatMs(value) {
  const normalized = Number(value);
  if (!Number.isFinite(normalized)) {
    return '-';
  }
  if (normalized >= 100) {
    return `${Math.round(normalized)}ms`;
  }
  if (normalized >= 10) {
    return `${normalized.toFixed(1)}ms`;
  }
  return `${normalized.toFixed(2)}ms`;
}

function formatPct(value) {
  const normalized = Number(value);
  if (!Number.isFinite(normalized)) {
    return '-';
  }
  return `${(normalized * 100).toFixed(1)}%`;
}

function formatMb(value, { signed = false } = {}) {
  const normalized = Number(value);
  if (!Number.isFinite(normalized)) {
    return '-';
  }
  const mb = normalized / (1024 * 1024);
  if (signed) {
    const prefix = mb > 0 ? '+' : '';
    return `${prefix}${mb.toFixed(1)}MB`;
  }
  return `${mb.toFixed(1)}MB`;
}

function formatTimestamp(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return '-';
  }
  const date = new Date(unixMs);
  return date.toLocaleTimeString();
}

export function DashboardPerfPanel() {
  const [expanded, setExpanded] = useState(false);
  const [snapshot, setSnapshot] = useState(() => resolvePerfSnapshot());

  useEffect(() => {
    const refresh = () => {
      setSnapshot(resolvePerfSnapshot());
    };
    refresh();
    const intervalId = window.setInterval(refresh, PERF_POLL_INTERVAL_MS);
    return () => {
      window.clearInterval(intervalId);
    };
  }, []);

  const rows = useMemo(() => {
    const transactionCommitSamples = snapshot?.transactionCommit?.samples ?? {};
    const scrollHandlerSamples = snapshot?.scroll?.handler?.samples ?? {};
    const scrollCommitLatencySamples = snapshot?.scroll?.commitLatency?.samples ?? {};
    const frame = snapshot?.frame ?? {};
    const memory = snapshot?.memory ?? {};
    const stream = memory.stream ?? {};
    return [
      {
        id: 'commit-p95',
        label: 'Tx Commit p95',
        value: formatMs(transactionCommitSamples.p95Ms),
      },
      {
        id: 'commit-avg',
        label: 'Tx Commit avg',
        value: formatMs(transactionCommitSamples.averageMs),
      },
      {
        id: 'scroll-handler-p95',
        label: 'Scroll Handler p95',
        value: formatMs(scrollHandlerSamples.p95Ms),
      },
      {
        id: 'scroll-latency-p95',
        label: 'Scroll->Commit p95',
        value: formatMs(scrollCommitLatencySamples.p95Ms),
      },
      {
        id: 'frame-dropped',
        label: 'Dropped Frames',
        value: formatPct(frame.ratio),
      },
      {
        id: 'fps',
        label: 'Rolling FPS',
        value: Number.isFinite(frame.rollingFps) ? Math.round(frame.rollingFps).toString() : '-',
      },
      {
        id: 'heap-latest',
        label: 'Heap (latest)',
        value: formatMb(memory.heap?.latestBytes),
      },
      {
        id: 'page-latest',
        label: 'Page Mem (latest)',
        value: formatMb(memory.page?.latestBytes),
      },
      {
        id: 'page-delta',
        label: 'Page Mem (delta)',
        value: formatMb(memory.page?.deltaBytes, { signed: true }),
      },
      {
        id: 'dom-nodes',
        label: 'DOM Nodes',
        value: Number.isFinite(memory.domNodes?.latestBytes)
          ? Math.round(memory.domNodes.latestBytes).toLocaleString()
          : '-',
      },
      {
        id: 'tx-rows',
        label: 'Tx Rows',
        value: Number.isFinite(stream.transactionRows)
          ? String(stream.transactionRows)
          : '-',
      },
      {
        id: 'pending-total',
        label: 'Pending Deltas',
        value: Number.isFinite(stream.pendingTransactions)
          ? String(
            stream.pendingTransactions + stream.pendingFeatures + stream.pendingOpportunities,
          )
          : '-',
      },
    ];
  }, [snapshot]);

  return (
    <aside className="fixed bottom-3 right-3 z-[60] w-[21rem] max-w-[92vw]">
      <div className="news-mono border-2 border-zinc-900 bg-[#fffdf7]/95 text-[11px] uppercase tracking-[0.1em] shadow-lg">
        <button
          type="button"
          className="flex w-full items-center justify-between gap-2 border-b border-zinc-900 px-3 py-2 text-left"
          onClick={() => setExpanded((current) => !current)}
          aria-expanded={expanded}
        >
          <span>Dev Perf Telemetry</span>
          <span>{expanded ? 'Hide' : 'Show'}</span>
        </button>
        {expanded ? (
          <div className="space-y-1 px-3 py-2">
            {rows.map((row) => (
              <div key={row.id} className="flex items-center justify-between gap-2">
                <span className="text-zinc-700">{row.label}</span>
                <span className="font-bold text-zinc-900">{row.value}</span>
              </div>
            ))}
            <div className="mt-2 border-t border-zinc-900 pt-1 text-[10px] text-zinc-700">
              <div>
                Memory: {snapshot?.memory?.analysis?.level ?? '-'} · {snapshot?.memory?.analysis?.summary ?? '-'}
              </div>
              Updated {formatTimestamp(snapshot?.updatedAtUnixMs)}
            </div>
          </div>
        ) : null}
      </div>
    </aside>
  );
}
