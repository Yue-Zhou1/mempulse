import { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import { resolveApiBase } from './api-base.js';
import { mergeTransactionHistory } from './tx-history.js';
import { BlurFade } from './components/magicui/blur-fade.jsx';
import { NumberTicker } from './components/magicui/number-ticker.jsx';
import { ShineBorder } from './components/magicui/shine-border.jsx';
import { cn } from './lib/utils.js';

const refreshIntervalMs = 2000;
const txPollLimit = 250;
const featurePollLimit = 600;
const maxTransactionHistory = 1500;
const maxRenderedTransactions = 300;
const storageKey = 'vizApiBase';

function getStoredApiBase() {
  try {
    return window.localStorage.getItem(storageKey);
  } catch {
    return null;
  }
}

function setStoredApiBase(value) {
  try {
    if (value) {
      window.localStorage.setItem(storageKey, value);
    }
  } catch {
    // Ignore storage errors for private/incognito contexts.
  }
}

function shortHex(value, head = 12, tail = 10) {
  if (!value || value.length <= head + tail + 3) {
    return value ?? '-';
  }
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

function formatTime(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return '-';
  }
  return new Date(unixMs).toLocaleString();
}

function formatRelativeTime(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return 'unknown';
  }
  const deltaMs = Date.now() - unixMs;
  const deltaSec = Math.max(0, Math.floor(deltaMs / 1000));
  if (deltaSec < 60) {
    return `${deltaSec}s ago`;
  }
  if (deltaSec < 3600) {
    return `${Math.floor(deltaSec / 60)}m ago`;
  }
  if (deltaSec < 86400) {
    return `${Math.floor(deltaSec / 3600)}h ago`;
  }
  return `${Math.floor(deltaSec / 86400)}d ago`;
}

async function fetchJson(apiBase, path) {
  const response = await fetch(`${apiBase}${path}`);
  if (!response.ok) {
    throw new Error(`failed ${path}: HTTP ${response.status}`);
  }
  return response.json();
}

function SidebarNavRow({ label, value, active = false }) {
  return (
    <button
      type="button"
      className={cn(
        'flex w-full items-center justify-between rounded-lg px-3 py-2 text-sm font-medium transition-colors',
        active
          ? 'bg-zinc-900 text-zinc-50'
          : 'text-zinc-700 hover:bg-zinc-200 hover:text-zinc-900',
      )}
    >
      <span>{label}</span>
      <span className={cn('tabular-nums', active ? 'text-zinc-300' : 'text-zinc-500')}>
        {value}
      </span>
    </button>
  );
}

function MetricCard({ label, value, accent }) {
  return (
    <div className="relative overflow-hidden rounded-xl border border-zinc-300 bg-white p-3">
      <ShineBorder borderWidth={1} duration={16} shineColor={[accent, '#ffffff', accent]} />
      <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">{label}</div>
      <div className="mt-1 text-2xl font-semibold text-zinc-900">
        <NumberTicker value={value} />
      </div>
    </div>
  );
}

export default function App() {
  const apiConfig = resolveApiBase({
    search: window.location.search,
    storedApiBase: getStoredApiBase(),
    protocol: window.location.protocol,
    hostname: window.location.hostname,
  });

  if (apiConfig.persistApiBase) {
    setStoredApiBase(apiConfig.persistApiBase);
  }

  const apiBase = apiConfig.apiBase;

  const [statusMessage, setStatusMessage] = useState('Connecting to API...');
  const [hasError, setHasError] = useState(false);
  const [query, setQuery] = useState('');
  const [timelineIndex, setTimelineIndex] = useState(0);
  const [followLatest, setFollowLatest] = useState(true);
  const [selectedHash, setSelectedHash] = useState(null);
  const [dialogHash, setDialogHash] = useState(null);
  const [dialogError, setDialogError] = useState('');
  const [dialogLoading, setDialogLoading] = useState(false);

  const [replayFrames, setReplayFrames] = useState([]);
  const [propagationEdges, setPropagationEdges] = useState([]);
  const [featureSummaryRows, setFeatureSummaryRows] = useState([]);
  const [featureDetailRows, setFeatureDetailRows] = useState([]);
  const [recentTxRows, setRecentTxRows] = useState([]);
  const [transactionRows, setTransactionRows] = useState([]);
  const [transactionDetailsByHash, setTransactionDetailsByHash] = useState({});
  const transactionRowsRef = useRef([]);

  useEffect(() => {
    transactionRowsRef.current = transactionRows;
  }, [transactionRows]);

  useEffect(() => {
    let mounted = true;

    const loadData = async () => {
      if (!mounted) {
        return;
      }

      setStatusMessage(`Loading from ${apiBase} ...`);
      setHasError(false);

      try {
        const [txRecent, replay, propagation, featureSummary, featureDetails] =
          await Promise.all([
            fetchJson(apiBase, `/transactions?limit=${txPollLimit}`),
            fetchJson(apiBase, '/replay'),
            fetchJson(apiBase, '/propagation'),
            fetchJson(apiBase, '/features'),
            fetchJson(apiBase, `/features/recent?limit=${featurePollLimit}`),
          ]);

        if (!mounted) {
          return;
        }

        const nextTransactions = mergeTransactionHistory(
          transactionRowsRef.current,
          txRecent,
          maxTransactionHistory,
        );
        transactionRowsRef.current = nextTransactions;

        setRecentTxRows(txRecent);
        setTransactionRows(nextTransactions);
        setReplayFrames(replay);
        setPropagationEdges(propagation);
        setFeatureSummaryRows(featureSummary);
        setFeatureDetailRows(featureDetails);

        setTimelineIndex((current) => {
          if (!replay.length) {
            return 0;
          }
          if (followLatest) {
            return replay.length - 1;
          }
          return Math.min(current, replay.length - 1);
        });

        setSelectedHash((current) => {
          if (current && nextTransactions.some((row) => row.hash === current)) {
            return current;
          }
          return nextTransactions[0]?.hash ?? null;
        });

        const lastUpdated = new Date().toLocaleTimeString();
        setStatusMessage(
          `Connected · replay=${replay.length} · propagation=${propagation.length} · features=${featureDetails.length} · tx=${nextTransactions.length}/${maxTransactionHistory} · ${lastUpdated}`,
        );
      } catch (error) {
        if (!mounted) {
          return;
        }
        setHasError(true);
        setStatusMessage(`API unavailable: ${error.message}`);
      }
    };

    loadData();
    const timer = window.setInterval(loadData, refreshIntervalMs);

    return () => {
      mounted = false;
      window.clearInterval(timer);
    };
  }, [apiBase, followLatest]);

  useEffect(() => {
    if (!dialogHash) {
      return undefined;
    }
    if (transactionDetailsByHash[dialogHash]) {
      return undefined;
    }

    let cancelled = false;
    setDialogLoading(true);
    setDialogError('');

    fetchJson(apiBase, `/transactions/${encodeURIComponent(dialogHash)}`)
      .then((detail) => {
        if (cancelled) {
          return;
        }
        setTransactionDetailsByHash((current) => ({
          ...current,
          [dialogHash]: detail,
        }));
        setDialogLoading(false);
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        setDialogLoading(false);
        setDialogError(`Failed to load tx details: ${error.message}`);
      });

    return () => {
      cancelled = true;
    };
  }, [apiBase, dialogHash, transactionDetailsByHash]);

  useEffect(() => {
    if (!dialogHash) {
      return undefined;
    }

    const onKeyDown = (event) => {
      if (event.key === 'Escape') {
        setDialogHash(null);
        setDialogError('');
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [dialogHash]);

  const featureByHash = useMemo(() => {
    const map = new Map();
    for (const row of featureDetailRows) {
      map.set(row.hash, row);
    }
    return map;
  }, [featureDetailRows]);

  const filteredTransactions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return transactionRows;
    }

    return transactionRows.filter((row) => {
      const feature = featureByHash.get(row.hash);
      return (
        row.hash?.toLowerCase().includes(needle) ||
        row.peer?.toLowerCase().includes(needle) ||
        row.sender?.toLowerCase().includes(needle) ||
        String(row.nonce ?? '').includes(needle) ||
        feature?.protocol?.toLowerCase().includes(needle) ||
        feature?.category?.toLowerCase().includes(needle)
      );
    });
  }, [featureByHash, query, transactionRows]);

  const selectedTransaction = useMemo(
    () => transactionRows.find((row) => row.hash === selectedHash) ?? null,
    [selectedHash, transactionRows],
  );
  const selectedDetail = useMemo(() => {
    if (!selectedTransaction) {
      return null;
    }
    return transactionDetailsByHash[selectedTransaction.hash] ?? null;
  }, [selectedTransaction, transactionDetailsByHash]);

  const selectedFeature = useMemo(() => {
    if (!selectedTransaction) {
      return null;
    }
    return featureByHash.get(selectedTransaction.hash) ?? null;
  }, [featureByHash, selectedTransaction]);

  const selectedRecent = useMemo(() => {
    if (!selectedTransaction) {
      return null;
    }
    return recentTxRows.find((row) => row.hash === selectedTransaction.hash) ?? null;
  }, [recentTxRows, selectedTransaction]);

  const dialogTransaction = useMemo(() => {
    if (!dialogHash) {
      return null;
    }
    return transactionRows.find((row) => row.hash === dialogHash) ?? null;
  }, [dialogHash, transactionRows]);

  const dialogDetail = useMemo(() => {
    if (!dialogHash) {
      return null;
    }
    return transactionDetailsByHash[dialogHash] ?? null;
  }, [dialogHash, transactionDetailsByHash]);

  const dialogFeature = useMemo(() => {
    if (!dialogHash) {
      return null;
    }
    return featureByHash.get(dialogHash) ?? null;
  }, [dialogHash, featureByHash]);

  const currentFrame = replayFrames[timelineIndex] ?? null;
  const visibleTransactions = filteredTransactions.slice(0, maxRenderedTransactions);

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_20%_10%,#e2e8f0_0%,#f8fafc_46%,#f1f5f9_100%)] p-4 text-zinc-900">
      <div className="mx-auto h-[calc(100vh-2rem)] max-w-[1700px] overflow-hidden rounded-2xl border border-zinc-300/90 bg-zinc-100 shadow-[0_20px_80px_-40px_rgba(15,23,42,0.55)]">
        <div className="grid h-full grid-cols-1 lg:grid-cols-[260px_minmax(360px,1fr)_minmax(440px,1fr)]">
          <aside className="overflow-auto border-b border-zinc-300 bg-zinc-100 p-4 lg:border-b-0 lg:border-r">
            <BlurFade inView className="space-y-4">
              <div className="rounded-xl border border-zinc-300 bg-white p-3">
                <div className="text-sm text-zinc-500">Operator</div>
                <div className="mt-1 text-base font-semibold">prototype03 mempool desk</div>
                <div className="mt-2 text-xs text-zinc-500">{apiBase}</div>
              </div>

              <div className="space-y-1">
                <SidebarNavRow label="All mail" value={filteredTransactions.length} active />
                <SidebarNavRow label="Unread" value={Math.max(0, filteredTransactions.length - 3)} />
                <SidebarNavRow label="Replay" value={replayFrames.length} />
                <SidebarNavRow label="Propagation" value={propagationEdges.length} />
                <SidebarNavRow label="Features" value={featureDetailRows.length} />
              </div>

              <div className="grid grid-cols-2 gap-2">
                <MetricCard label="Pending" value={currentFrame?.pending_count ?? 0} accent="#22d3ee" />
                <MetricCard label="Seq" value={currentFrame?.seq_hi ?? 0} accent="#60a5fa" />
                <MetricCard label="Edges" value={propagationEdges.length} accent="#34d399" />
                <MetricCard label="Tx Seen" value={transactionRows.length} accent="#f59e0b" />
              </div>
            </BlurFade>
          </aside>

          <main className="flex min-h-0 flex-col border-b border-zinc-300 bg-zinc-100 lg:border-b-0 lg:border-r">
            <div className="border-b border-zinc-300 bg-zinc-100/70 px-4 py-3 backdrop-blur">
              <div className="flex flex-wrap items-center gap-2">
                <h1 className="mr-auto text-2xl font-semibold tracking-tight">Inbox</h1>
                <button
                  type="button"
                  className="rounded-lg border border-zinc-300 bg-white px-3 py-1.5 text-sm font-medium text-zinc-700"
                >
                  All mail
                </button>
                <button
                  type="button"
                  className="rounded-lg border border-transparent px-3 py-1.5 text-sm font-medium text-zinc-500"
                >
                  Unread
                </button>
              </div>
              <div className="mt-3 flex items-center gap-2">
                <input
                  type="search"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Search hash, sender, protocol, category"
                  className="w-full rounded-lg border border-zinc-300 bg-white px-3 py-2 text-sm outline-none transition focus:border-zinc-500"
                />
              </div>
              <div className={cn('mt-2 text-xs', hasError ? 'text-rose-600' : 'text-zinc-500')}>
                {statusMessage}
              </div>
              <div className="mt-1 text-[11px] text-zinc-500">
                Retaining latest {maxTransactionHistory} transactions; rendering up to{' '}
                {maxRenderedTransactions} rows.
              </div>
            </div>

            <div className="min-h-0 flex-1 overflow-auto p-3">
              <AnimatePresence initial={false}>
                {visibleTransactions.map((row, index) => {
                  const feature = featureByHash.get(row.hash);
                  const isActive = row.hash === selectedHash;
                  return (
                    <motion.button
                      type="button"
                      key={row.hash}
                      onClick={() => {
                        setSelectedHash(row.hash);
                        setDialogHash(row.hash);
                        setDialogError('');
                      }}
                      initial={{ opacity: 0, y: 8 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: Math.min(index * 0.015, 0.25), duration: 0.25 }}
                      className={cn(
                        'group relative mb-2 block w-full overflow-hidden rounded-xl border p-3 text-left transition',
                        isActive
                          ? 'border-zinc-900 bg-zinc-900 text-zinc-100 shadow-lg'
                          : 'border-zinc-300 bg-white text-zinc-800 hover:border-zinc-400 hover:shadow',
                      )}
                    >
                      {isActive ? (
                        <ShineBorder borderWidth={1} duration={11} shineColor={['#fafafa', '#52525b', '#fafafa']} />
                      ) : null}
                      <div className="flex items-start gap-3">
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center justify-between gap-2">
                            <div className="truncate text-sm font-semibold">
                              {shortHex(row.sender, 12, 6)}
                            </div>
                            <div className={cn('text-xs', isActive ? 'text-zinc-400' : 'text-zinc-500')}>
                              {formatRelativeTime(row.seen_unix_ms)}
                            </div>
                          </div>
                          <div
                            className={cn(
                              'mt-0.5 truncate text-xs',
                              isActive ? 'text-zinc-300' : 'text-zinc-500',
                            )}
                          >
                            hash {shortHex(row.hash, 16, 10)} · source {row.source_id}
                          </div>
                          <div className="mt-2 flex flex-wrap gap-1.5">
                            <span
                              className={cn(
                                'rounded-full border px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide',
                                isActive
                                  ? 'border-zinc-600 text-zinc-300'
                                  : 'border-zinc-300 text-zinc-600',
                              )}
                            >
                              nonce {row.nonce ?? '-'}
                            </span>
                            <span
                              className={cn(
                                'rounded-full border px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide',
                                isActive
                                  ? 'border-zinc-600 text-zinc-300'
                                  : 'border-zinc-300 text-zinc-600',
                              )}
                            >
                              type {row.tx_type}
                            </span>
                            {feature ? (
                              <>
                                <span
                                  className={cn(
                                    'rounded-full border px-2 py-0.5 text-[11px] font-semibold',
                                    isActive
                                      ? 'border-cyan-600/80 text-cyan-200'
                                      : 'border-cyan-200 text-cyan-700',
                                  )}
                                >
                                  {feature.protocol}
                                </span>
                                <span
                                  className={cn(
                                    'rounded-full border px-2 py-0.5 text-[11px] font-semibold',
                                    isActive
                                      ? 'border-blue-600/80 text-blue-200'
                                      : 'border-blue-200 text-blue-700',
                                  )}
                                >
                                  {feature.category}
                                </span>
                              </>
                            ) : null}
                          </div>
                        </div>
                      </div>
                    </motion.button>
                  );
                })}
              </AnimatePresence>

              {filteredTransactions.length > maxRenderedTransactions ? (
                <div className="mb-3 rounded-lg border border-zinc-300 bg-white px-3 py-2 text-xs text-zinc-500">
                  Showing {maxRenderedTransactions} of {filteredTransactions.length} matching rows.
                </div>
              ) : null}

              {filteredTransactions.length === 0 ? (
                <div className="rounded-xl border border-dashed border-zinc-300 bg-white p-8 text-center text-sm text-zinc-500">
                  No rows match your search.
                </div>
              ) : null}
            </div>
          </main>

          <section className="flex min-h-0 flex-col bg-zinc-100">
            <div className="border-b border-zinc-300 bg-zinc-100/70 px-4 py-3 backdrop-blur">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <div className="text-lg font-semibold">
                    {selectedTransaction ? shortHex(selectedTransaction.sender, 16, 8) : 'No selection'}
                  </div>
                  <div className="text-xs text-zinc-500">
                    {selectedTransaction
                      ? `${selectedDetail ? `Peer ${selectedDetail.peer}` : `Source ${selectedTransaction.source_id}`} · ${formatTime(selectedDetail?.first_seen_unix_ms ?? selectedTransaction.seen_unix_ms)}`
                      : 'Select a transaction from the middle pane'}
                  </div>
                </div>
                <div className="text-xs text-zinc-500">
                  {selectedTransaction
                    ? formatRelativeTime(selectedDetail?.first_seen_unix_ms ?? selectedTransaction.seen_unix_ms)
                    : ''}
                </div>
              </div>
            </div>

            <div className="min-h-0 flex-1 overflow-auto p-4">
              <BlurFade inView className="space-y-4">
                <div className="rounded-xl border border-zinc-300 bg-white p-4">
                  <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                    Transaction Snapshot
                  </div>
                  <div className="mt-3 grid grid-cols-1 gap-3 text-sm md:grid-cols-2">
                    <div>
                      <div className="text-xs text-zinc-500">Hash</div>
                      <div className="font-mono text-[13px] text-zinc-900">
                        {selectedTransaction?.hash ?? '-'}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Seen At</div>
                      <div>
                        {selectedTransaction
                          ? formatTime(selectedDetail?.first_seen_unix_ms ?? selectedTransaction.seen_unix_ms)
                          : '-'}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Sender</div>
                      <div className="font-mono text-[13px] text-zinc-900">
                        {selectedTransaction?.sender ?? selectedDetail?.sender ?? '-'}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Nonce</div>
                      <div>{selectedTransaction?.nonce ?? selectedDetail?.nonce ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Tx Type</div>
                      <div>{selectedTransaction?.tx_type ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Source</div>
                      <div>{selectedTransaction?.source_id ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Seen Count</div>
                      <div>{selectedDetail?.seen_count ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Raw Payload Size</div>
                      <div>{selectedDetail?.raw_tx_len ?? '-'} bytes</div>
                    </div>
                  </div>
                </div>

                <div className="rounded-xl border border-zinc-300 bg-white p-4">
                  <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                    Feature Engine
                  </div>
                  <div className="mt-3 grid grid-cols-2 gap-3 text-sm">
                    <div>
                      <div className="text-xs text-zinc-500">Protocol</div>
                      <div>{selectedFeature?.protocol ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Category</div>
                      <div>{selectedFeature?.category ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">MEV Score</div>
                      <div>{selectedFeature?.mev_score ?? '-'}</div>
                    </div>
                    <div>
                      <div className="text-xs text-zinc-500">Urgency Score</div>
                      <div>{selectedFeature?.urgency_score ?? '-'}</div>
                    </div>
                    <div className="col-span-2">
                      <div className="text-xs text-zinc-500">Method Selector</div>
                      <div className="font-mono text-[13px]">{selectedFeature?.method_selector ?? '-'}</div>
                    </div>
                  </div>
                </div>

                <div className="rounded-xl border border-zinc-300 bg-white p-4">
                  <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                    Replay Timeline
                  </div>
                  <div className="mt-3">
                    <input
                      type="range"
                      min="0"
                      max={Math.max(0, replayFrames.length - 1)}
                      value={timelineIndex}
                      onChange={(event) => {
                        setTimelineIndex(Number(event.target.value));
                        setFollowLatest(false);
                      }}
                      className="w-full"
                    />
                    <div className="mt-2 flex items-center justify-between text-xs text-zinc-500">
                      <div>
                        frame {replayFrames.length ? timelineIndex + 1 : 0}/{replayFrames.length}
                      </div>
                      <div>
                        seq {currentFrame?.seq_hi ?? '-'} · pending {currentFrame?.pending_count ?? 0}
                      </div>
                    </div>
                    <div className="mt-3 flex items-center justify-between gap-3">
                      <button
                        type="button"
                        onClick={() => {
                          setFollowLatest(true);
                          if (replayFrames.length) {
                            setTimelineIndex(replayFrames.length - 1);
                          }
                        }}
                        className={cn(
                          'rounded-lg border px-3 py-1.5 text-xs font-semibold uppercase tracking-wide transition',
                          followLatest
                            ? 'border-zinc-900 bg-zinc-900 text-white'
                            : 'border-zinc-300 bg-zinc-50 text-zinc-700 hover:bg-zinc-200',
                        )}
                      >
                        Follow live
                      </button>
                      <div className="text-xs text-zinc-500">
                        Feature rows loaded: {featureSummaryRows.length}
                      </div>
                    </div>
                  </div>
                </div>

                <div className="rounded-xl border border-zinc-300 bg-white p-4">
                  <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                    Thread Notes
                  </div>
                  <textarea
                    readOnly
                    value={selectedRecent
                      ? `Source ${selectedRecent.source_id} observed tx type ${selectedRecent.tx_type} at ${formatTime(selectedRecent.seen_unix_ms)}.`
                      : 'Select a transaction to inspect metadata in this pane.'}
                    className="mt-3 h-24 w-full resize-none rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-700"
                  />
                </div>
              </BlurFade>
            </div>
          </section>
        </div>
      </div>

      {dialogHash ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-950/50 p-4 backdrop-blur-sm"
          onClick={() => {
            setDialogHash(null);
            setDialogError('');
          }}
        >
          <div
            className="max-h-[90vh] w-full max-w-3xl overflow-auto rounded-2xl border border-zinc-300 bg-white p-5 shadow-2xl"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                  Transaction Detail
                </div>
                <div className="font-mono text-[13px] text-zinc-900">{dialogHash}</div>
              </div>
              <button
                type="button"
                onClick={() => {
                  setDialogHash(null);
                  setDialogError('');
                }}
                className="rounded-lg border border-zinc-300 px-3 py-1.5 text-xs font-semibold uppercase tracking-wide text-zinc-700 transition hover:bg-zinc-100"
              >
                Close
              </button>
            </div>

            <div className="mt-4 grid grid-cols-1 gap-3 text-sm md:grid-cols-2">
              <div>
                <div className="text-xs text-zinc-500">Sender</div>
                <div className="font-mono text-[13px]">{dialogTransaction?.sender ?? dialogDetail?.sender ?? '-'}</div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Nonce</div>
                <div>{dialogTransaction?.nonce ?? dialogDetail?.nonce ?? '-'}</div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Tx Type</div>
                <div>{dialogTransaction?.tx_type ?? '-'}</div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Source</div>
                <div>{dialogTransaction?.source_id ?? '-'}</div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Seen At</div>
                <div>
                  {formatTime(dialogDetail?.first_seen_unix_ms ?? dialogTransaction?.seen_unix_ms)}
                </div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Peer</div>
                <div>{dialogDetail?.peer ?? '-'}</div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Seen Count</div>
                <div>{dialogDetail?.seen_count ?? '-'}</div>
              </div>
              <div>
                <div className="text-xs text-zinc-500">Raw Payload Size</div>
                <div>{dialogDetail?.raw_tx_len ?? '-'} bytes</div>
              </div>
            </div>

            <div className="mt-4 rounded-xl border border-zinc-300 bg-zinc-50 p-3">
              <div className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                Feature Engine
              </div>
              <div className="mt-2 grid grid-cols-2 gap-3 text-sm">
                <div>
                  <div className="text-xs text-zinc-500">Protocol</div>
                  <div>{dialogFeature?.protocol ?? '-'}</div>
                </div>
                <div>
                  <div className="text-xs text-zinc-500">Category</div>
                  <div>{dialogFeature?.category ?? '-'}</div>
                </div>
                <div>
                  <div className="text-xs text-zinc-500">MEV Score</div>
                  <div>{dialogFeature?.mev_score ?? '-'}</div>
                </div>
                <div>
                  <div className="text-xs text-zinc-500">Urgency Score</div>
                  <div>{dialogFeature?.urgency_score ?? '-'}</div>
                </div>
                <div className="col-span-2">
                  <div className="text-xs text-zinc-500">Method Selector</div>
                  <div className="font-mono text-[13px]">{dialogFeature?.method_selector ?? '-'}</div>
                </div>
              </div>
            </div>

            {dialogLoading ? (
              <div className="mt-4 rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-xs text-zinc-600">
                Loading on-demand transaction detail...
              </div>
            ) : null}
            {dialogError ? (
              <div className="mt-4 rounded-lg border border-rose-300 bg-rose-50 px-3 py-2 text-xs text-rose-700">
                {dialogError}
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
