import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { resolveApiBase } from './api-base.js';
import { readWindowRuntimeConfig, resolveUiRuntimeConfig } from './ui-config.js';
import {
  createMarketStatsState,
  resolveMarketStatsSnapshot,
} from './market-stats.js';
import { mergeTransactionHistory } from './tx-history.js';
import { normalizeScreenId } from './screen-mode.js';
import { cn } from './lib/utils.js';

const snapshotFeatureLimit = 600;
const snapshotOppLimit = 600;
const snapshotReplayLimit = 1000;
const snapshotThrottleMs = 1200;
const streamBatchLimit = 512;
const streamIntervalMs = 200;
const streamInitialReconnectMs = 1000;
const streamMaxReconnectMs = 30000;
const archiveTxFetchLimit = 1500;
const archiveOppFetchLimit = 1500;
const archiveRefreshMs = 15000;
const archiveTxPageSize = 40;
const archiveOppPageSize = 20;
const storageKey = 'vizApiBase';
const tickerPageSize = 50;
const tickerPageLimit = 10;

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

function formatTickerTime(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return '----/--/-- --:--:--';
  }
  const date = new Date(unixMs);
  const yyyy = String(date.getFullYear());
  const mm = String(date.getMonth() + 1).padStart(2, '0');
  const dd = String(date.getDate()).padStart(2, '0');
  const hh = String(date.getHours()).padStart(2, '0');
  const mi = String(date.getMinutes()).padStart(2, '0');
  const ss = String(date.getSeconds()).padStart(2, '0');
  return `${yyyy}/${mm}/${dd} ${hh}:${mi}:${ss}`;
}

async function fetchJson(apiBase, path) {
  const response = await fetch(`${apiBase}${path}`);
  if (!response.ok) {
    throw new Error(`failed ${path}: HTTP ${response.status}`);
  }
  return response.json();
}

function buildDashboardSnapshotPath(txLimit) {
  const params = new URLSearchParams({
    tx_limit: String(txLimit),
    feature_limit: String(snapshotFeatureLimit),
    opp_limit: String(snapshotOppLimit),
    replay_limit: String(snapshotReplayLimit),
  });
  return `/dashboard/snapshot?${params.toString()}`;
}

function resolveStreamUrl(apiBase, afterSeqId) {
  const url = new URL(apiBase, window.location.href);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = `${url.pathname.replace(/\/+$/, '')}/stream`;
  const params = new URLSearchParams({
    limit: String(streamBatchLimit),
    interval_ms: String(streamIntervalMs),
  });
  if (Number.isFinite(afterSeqId) && afterSeqId > 0) {
    params.set('after', String(afterSeqId));
  }
  url.search = params.toString();
  return url.toString();
}

function classifyRisk(feature) {
  const score = feature?.mev_score ?? 0;
  if (score >= 80) {
    return {
      label: 'High',
      accent: 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]',
    };
  }
  if (score >= 45) {
    return {
      label: 'Medium',
      accent: 'border-zinc-900 text-zinc-800',
    };
  }
  return {
    label: 'Low',
    accent: 'border-zinc-700 text-zinc-700',
  };
}

function statusForRow(feature) {
  if (!feature) {
    return 'Pending';
  }
  if (feature.mev_score >= 85) {
    return 'Flagged';
  }
  if (feature.urgency_score >= 70) {
    return 'Processing';
  }
  return 'Completed';
}

function statusBadgeClass(status, inverted = false) {
  if (inverted) {
    return 'border-[#f7f1e6] text-[#f7f1e6]';
  }
  if (status === 'Completed') {
    return 'border-[#2e5f3a] bg-[#e7f1e8] text-[#244d30]';
  }
  if (status === 'Pending' || status === 'Processing') {
    return 'border-[#8a621f] bg-[#fff2dc] text-[#6d4c18]';
  }
  if (status === 'Flagged') {
    return 'border-[#7a2d2d] bg-[#fde8e8] text-[#6b2020]';
  }
  return 'border-zinc-900 text-zinc-800';
}

function riskBadgeClass(level, inverted = false) {
  if (inverted) {
    return 'border-[#f7f1e6] text-[#f7f1e6]';
  }
  if (level === 'Low') {
    return 'border-[#2e5f3a] bg-[#e7f1e8] text-[#244d30]';
  }
  if (level === 'Medium') {
    return 'border-[#8a621f] bg-[#fff2dc] text-[#6d4c18]';
  }
  if (level === 'High') {
    return 'border-[#7a2d2d] bg-[#fde8e8] text-[#6b2020]';
  }
  return 'border-zinc-900 text-zinc-800';
}

function opportunityCandidateTone(opportunity, active = false) {
  const strategy = String(opportunity?.strategy ?? '').toLowerCase();
  const category = String(opportunity?.category ?? '').toLowerCase();

  const palette = (() => {
    if (strategy.includes('sandwich')) {
      return {
        activeContainer: 'border-[#5f4413] bg-[#8a621f] text-[#fff8ee]',
        activeSubtle: 'text-[#f9e6c8]',
        idleContainer: 'border-[#8a621f] bg-[#fff2dc] text-[#4a3312] hover:bg-[#fae7c8]',
        idleSubtle: 'text-[#6d4c18]',
      };
    }
    if (strategy.includes('arb')) {
      return {
        activeContainer: 'border-[#1f4328] bg-[#2e5f3a] text-[#eef8f0]',
        activeSubtle: 'text-[#d6ecd8]',
        idleContainer: 'border-[#2e5f3a] bg-[#e7f1e8] text-[#1f3c27] hover:bg-[#d9ebdc]',
        idleSubtle: 'text-[#285033]',
      };
    }
    if (strategy.includes('backrun')) {
      return {
        activeContainer: 'border-[#1f3e6a] bg-[#315b8f] text-[#eef5ff]',
        activeSubtle: 'text-[#d7e6ff]',
        idleContainer: 'border-[#315b8f] bg-[#e7eef8] text-[#1f3657] hover:bg-[#dae5f5]',
        idleSubtle: 'text-[#294875]',
      };
    }
    if (strategy.includes('liquidation') || category.includes('liquidation')) {
      return {
        activeContainer: 'border-[#5a2121] bg-[#7a2d2d] text-[#fff2f2]',
        activeSubtle: 'text-[#f7d8d8]',
        idleContainer: 'border-[#7a2d2d] bg-[#fde8e8] text-[#4f1e1e] hover:bg-[#f9dada]',
        idleSubtle: 'text-[#6b2020]',
      };
    }
    if (category.includes('swap')) {
      return {
        activeContainer: 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]',
        activeSubtle: 'text-zinc-300',
        idleContainer: 'border-zinc-900 bg-[#fffdf7] text-zinc-900 hover:bg-zinc-100/60',
        idleSubtle: 'text-zinc-700',
      };
    }
    return {
      activeContainer: 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]',
      activeSubtle: 'text-zinc-300',
      idleContainer: 'border-zinc-900 bg-[#fffdf7] text-zinc-900 hover:bg-zinc-100/60',
      idleSubtle: 'text-zinc-700',
    };
  })();

  return active
    ? {
        container: palette.activeContainer,
        subtle: palette.activeSubtle,
      }
    : {
        container: palette.idleContainer,
        subtle: palette.idleSubtle,
      };
}

function opportunityRowKey(opportunity) {
  const txHash = String(opportunity?.tx_hash ?? '');
  const strategy = String(opportunity?.strategy ?? '');
  const detectedUnixMs = Number.isFinite(opportunity?.detected_unix_ms)
    ? String(opportunity.detected_unix_ms)
    : '';
  return `${txHash}::${strategy}::${detectedUnixMs}`;
}

function sparklinePath(values, width, height) {
  if (!values.length) {
    return '';
  }
  const max = Math.max(...values, 1);
  const step = values.length > 1 ? width / (values.length - 1) : width;
  return values
    .map((value, index) => {
      const x = index * step;
      const y = height - (value / max) * height;
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(2)} ${y.toFixed(2)}`;
    })
    .join(' ');
}

function paginationWindow(currentPage, totalPages, maxButtons = 5) {
  if (!Number.isFinite(totalPages) || totalPages <= 0) {
    return [1];
  }
  if (totalPages <= maxButtons) {
    return Array.from({ length: totalPages }, (_, index) => index + 1);
  }
  const half = Math.floor(maxButtons / 2);
  let start = Math.max(1, currentPage - half);
  let end = Math.min(totalPages, start + maxButtons - 1);
  start = Math.max(1, end - maxButtons + 1);
  const pages = [];
  for (let page = start; page <= end; page += 1) {
    pages.push(page);
  }
  return pages;
}

function easeOutCubic(progress) {
  return 1 - (1 - progress) ** 3;
}

function useRollingNumber(value, options = {}) {
  const durationMs = Number.isFinite(options.durationMs)
    ? Math.max(120, Math.floor(options.durationMs))
    : 520;
  const sanitizedTarget = Number.isFinite(value) ? value : 0;
  const [displayValue, setDisplayValue] = useState(sanitizedTarget);
  const displayValueRef = useRef(sanitizedTarget);
  const frameRef = useRef(null);

  useEffect(() => {
    displayValueRef.current = displayValue;
  }, [displayValue]);

  useEffect(() => {
    const fromValue = displayValueRef.current;
    if (sanitizedTarget <= fromValue) {
      if (frameRef.current) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      displayValueRef.current = sanitizedTarget;
      setDisplayValue(sanitizedTarget);
      return undefined;
    }

    let startTs = null;
    const delta = sanitizedTarget - fromValue;

    const tick = (ts) => {
      if (startTs == null) {
        startTs = ts;
      }
      const progress = Math.min(1, (ts - startTs) / durationMs);
      const nextValue = fromValue + delta * easeOutCubic(progress);
      displayValueRef.current = nextValue;
      setDisplayValue(nextValue);
      if (progress < 1) {
        frameRef.current = window.requestAnimationFrame(tick);
      } else {
        frameRef.current = null;
      }
    };

    frameRef.current = window.requestAnimationFrame(tick);
    return () => {
      if (frameRef.current) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [durationMs, sanitizedTarget]);

  return displayValue;
}

function RollingInt({ value, durationMs = 520, className }) {
  const display = useRollingNumber(value, { durationMs });
  return <span className={className}>{Math.round(display).toLocaleString()}</span>;
}

function RollingPercent({ value, durationMs = 520, className, suffix = '%' }) {
  const display = useRollingNumber(value, { durationMs });
  return (
    <span className={className}>
      {display.toFixed(1)}
      {suffix}
    </span>
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
  const uiConfig = useMemo(
    () =>
      resolveUiRuntimeConfig({
        runtime: readWindowRuntimeConfig(),
        env: import.meta.env,
      }),
    [],
  );
  const {
    snapshotTxLimit,
    detailCacheLimit,
    maxTransactionHistory,
    transactionRetentionMs,
    transactionRetentionMinutes,
  } = uiConfig;

  const [statusMessage, setStatusMessage] = useState('Connecting to API...');
  const [hasError, setHasError] = useState(false);
  const [activeScreen, setActiveScreen] = useState(() =>
    normalizeScreenId(window.location.hash.replace(/^#/, '')),
  );
  const [query, setQuery] = useState('');
  const [transactionPage, setTransactionPage] = useState(1);
  const [timelineIndex, setTimelineIndex] = useState(0);
  const [followLatest, setFollowLatest] = useState(true);
  const [selectedHash, setSelectedHash] = useState(null);
  const [selectedOpportunityKey, setSelectedOpportunityKey] = useState(null);
  const [dialogHash, setDialogHash] = useState(null);
  const [dialogError, setDialogError] = useState('');
  const [dialogLoading, setDialogLoading] = useState(false);

  const [replayFrames, setReplayFrames] = useState([]);
  const [propagationEdges, setPropagationEdges] = useState([]);
  const [opportunityRows, setOpportunityRows] = useState([]);
  const [archiveTxRows, setArchiveTxRows] = useState([]);
  const [archiveOppRows, setArchiveOppRows] = useState([]);
  const [archiveQuery, setArchiveQuery] = useState('');
  const [archiveLoading, setArchiveLoading] = useState(false);
  const [archiveError, setArchiveError] = useState('');
  const [archiveTxPage, setArchiveTxPage] = useState(1);
  const [archiveOppPage, setArchiveOppPage] = useState(1);
  const [selectedArchiveTxHash, setSelectedArchiveTxHash] = useState(null);
  const [selectedArchiveOppKey, setSelectedArchiveOppKey] = useState(null);
  const [featureSummaryRows, setFeatureSummaryRows] = useState([]);
  const [featureDetailRows, setFeatureDetailRows] = useState([]);
  const [recentTxRows, setRecentTxRows] = useState([]);
  const [transactionRows, setTransactionRows] = useState([]);
  const [marketStats, setMarketStats] = useState(() => createMarketStatsState());
  const [transactionDetailsByHash, setTransactionDetailsByHash] = useState({});
  const transactionRowsRef = useRef([]);
  const followLatestRef = useRef(followLatest);
  const latestSeqIdRef = useRef(0);
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef(null);
  const snapshotTimerRef = useRef(null);
  const streamSocketRef = useRef(null);
  const snapshotInFlightRef = useRef(false);
  const nextSnapshotAtRef = useRef(0);
  const archiveRequestSeqRef = useRef(0);

  useEffect(() => {
    transactionRowsRef.current = transactionRows;
  }, [transactionRows]);

  useEffect(() => {
    followLatestRef.current = followLatest;
  }, [followLatest]);

  useEffect(() => {
    if (followLatest) {
      setTransactionPage(1);
    }
  }, [followLatest, transactionRows]);

  useEffect(() => {
    setTransactionPage(1);
  }, [query]);

  const closeDialog = useCallback(() => {
    setDialogHash(null);
    setDialogError('');
  }, []);

  const openTransactionByHash = useCallback((hash) => {
    if (!hash) {
      return;
    }
    setSelectedHash(hash);
    setDialogHash(hash);
    setDialogError('');
  }, []);

  const onTickerListClick = useCallback(
    (event) => {
      const node = event.target;
      const listElement = event.currentTarget;
      if (!(node instanceof Element) || !(listElement instanceof Element)) {
        return;
      }
      const rowElement = node.closest('tr[data-tx-hash]');
      if (!rowElement || !listElement.contains(rowElement)) {
        return;
      }
      const hash = rowElement?.getAttribute('data-tx-hash');
      if (!hash) {
        return;
      }
      openTransactionByHash(hash);
    },
    [openTransactionByHash],
  );

  const openOpportunityByKey = useCallback((key) => {
    if (!key) {
      return;
    }
    setSelectedOpportunityKey(key);
  }, []);

  const onOpportunityListClick = useCallback(
    (event) => {
      const node = event.target;
      if (!(node instanceof Element)) {
        return;
      }
      const rowButton = node.closest('button[data-opportunity-key]');
      const key = rowButton?.getAttribute('data-opportunity-key');
      if (!key) {
        return;
      }
      openOpportunityByKey(key);
    },
    [openOpportunityByKey],
  );

  const onArchiveTxListClick = useCallback((event) => {
    const node = event.target;
    if (!(node instanceof Element)) {
      return;
    }
    const rowButton = node.closest('button[data-archive-tx-hash]');
    const hash = rowButton?.getAttribute('data-archive-tx-hash');
    if (!hash) {
      return;
    }
    setSelectedArchiveTxHash(hash);
  }, []);

  const onArchiveOppListClick = useCallback((event) => {
    const node = event.target;
    if (!(node instanceof Element)) {
      return;
    }
    const rowButton = node.closest('button[data-archive-opp-key]');
    const key = rowButton?.getAttribute('data-archive-opp-key');
    if (!key) {
      return;
    }
    setSelectedArchiveOppKey(key);
  }, []);

  const refreshArchives = useCallback(async () => {
    const requestSeq = archiveRequestSeqRef.current + 1;
    archiveRequestSeqRef.current = requestSeq;
    setArchiveLoading(true);
    setArchiveError('');

    try {
      const [txPayload, oppPayload] = await Promise.all([
        fetchJson(apiBase, `/transactions/all?limit=${archiveTxFetchLimit}`),
        fetchJson(apiBase, `/opps/recent?limit=${archiveOppFetchLimit}`),
      ]);
      if (archiveRequestSeqRef.current !== requestSeq) {
        return;
      }

      const nextTxRows = Array.isArray(txPayload) ? [...txPayload] : [];
      const nextOppRows = Array.isArray(oppPayload) ? [...oppPayload] : [];

      nextTxRows.sort((left, right) =>
        Number(right?.first_seen_unix_ms ?? 0) - Number(left?.first_seen_unix_ms ?? 0),
      );
      nextOppRows.sort((left, right) =>
        Number(right?.detected_unix_ms ?? 0) - Number(left?.detected_unix_ms ?? 0),
      );

      setArchiveTxRows(nextTxRows);
      setArchiveOppRows(nextOppRows);
      setSelectedArchiveTxHash((current) => {
        if (current && nextTxRows.some((row) => row.hash === current)) {
          return current;
        }
        return nextTxRows[0]?.hash ?? null;
      });
      setSelectedArchiveOppKey((current) => {
        if (current && nextOppRows.some((row) => opportunityRowKey(row) === current)) {
          return current;
        }
        return nextOppRows[0] ? opportunityRowKey(nextOppRows[0]) : null;
      });
    } catch (error) {
      if (archiveRequestSeqRef.current !== requestSeq) {
        return;
      }
      setArchiveError(`Archive sync failed: ${error.message}`);
    } finally {
      if (archiveRequestSeqRef.current === requestSeq) {
        setArchiveLoading(false);
      }
    }
  }, [apiBase]);

  useEffect(() => {
    const onHashChange = () => {
      setActiveScreen(normalizeScreenId(window.location.hash.replace(/^#/, '')));
    };
    window.addEventListener('hashchange', onHashChange);
    return () => {
      window.removeEventListener('hashchange', onHashChange);
    };
  }, []);

  useEffect(() => {
    const expectedHash = `#${activeScreen}`;
    if (window.location.hash !== expectedHash) {
      window.history.replaceState(null, '', expectedHash);
    }
  }, [activeScreen]);

  useEffect(() => {
    if (activeScreen !== 'replay') {
      return undefined;
    }
    refreshArchives();
    const refreshTimer = window.setInterval(() => {
      refreshArchives();
    }, archiveRefreshMs);
    return () => {
      window.clearInterval(refreshTimer);
    };
  }, [activeScreen, refreshArchives]);

  useEffect(() => {
    let cancelled = false;
    const snapshotPath = buildDashboardSnapshotPath(snapshotTxLimit);

    const cleanupSocket = () => {
      if (streamSocketRef.current) {
        const socket = streamSocketRef.current;
        streamSocketRef.current = null;
        socket.onopen = null;
        socket.onmessage = null;
        socket.onclose = null;
        socket.onerror = null;
        socket.close();
      }
    };

    const clearReconnectTimer = () => {
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
    };

    const clearSnapshotTimer = () => {
      if (snapshotTimerRef.current) {
        window.clearTimeout(snapshotTimerRef.current);
        snapshotTimerRef.current = null;
      }
    };

    const applySnapshot = (snapshot, sourceLabel) => {
      const replay = Array.isArray(snapshot?.replay) ? snapshot.replay : [];
      const propagation = Array.isArray(snapshot?.propagation) ? snapshot.propagation : [];
      const opportunities = Array.isArray(snapshot?.opportunities) ? snapshot.opportunities : [];
      const featureSummary = Array.isArray(snapshot?.feature_summary)
        ? snapshot.feature_summary
        : [];
      const featureDetails = Array.isArray(snapshot?.feature_details)
        ? snapshot.feature_details
        : [];
      const txRecent = Array.isArray(snapshot?.transactions) ? snapshot.transactions : [];

      const nextTransactions = mergeTransactionHistory(
        transactionRowsRef.current,
        txRecent,
        maxTransactionHistory,
        {
          nowUnixMs: Date.now(),
          maxAgeMs: transactionRetentionMs,
        },
      );
      transactionRowsRef.current = nextTransactions;

      setRecentTxRows(txRecent);
      setTransactionRows(nextTransactions);
      setTransactionDetailsByHash((current) => {
        const liveHashes = new Set(nextTransactions.map((row) => row.hash));
        const next = {};
        for (const [hash, detail] of Object.entries(current)) {
          if (liveHashes.has(hash)) {
            next[hash] = detail;
          }
        }
        return next;
      });
      setReplayFrames(replay);
      setPropagationEdges(propagation);
      setOpportunityRows(opportunities);
      setFeatureSummaryRows(featureSummary);
      setFeatureDetailRows(featureDetails);
      setMarketStats((current) =>
        resolveMarketStatsSnapshot(snapshot?.market_stats, current),
      );

      setTimelineIndex((current) => {
        if (!replay.length) {
          return 0;
        }
        if (followLatestRef.current) {
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
      setSelectedOpportunityKey((current) => {
        if (current && opportunities.some((row) => opportunityRowKey(row) === current)) {
          return current;
        }
        return opportunities[0] ? opportunityRowKey(opportunities[0]) : null;
      });

      const latestSeqId = Number.isFinite(snapshot?.latest_seq_id)
        ? snapshot.latest_seq_id
        : replay[replay.length - 1]?.seq_hi ?? 0;
      latestSeqIdRef.current = Math.max(latestSeqIdRef.current, latestSeqId);

      const lastUpdated = new Date().toLocaleTimeString();
      setStatusMessage(
        `Connected(${sourceLabel}) · replay=${replay.length} · opps=${opportunities.length} · propagation=${propagation.length} · features=${featureDetails.length} · tx=${nextTransactions.length}/${maxTransactionHistory} · window=${transactionRetentionMinutes}m · ${lastUpdated}`,
      );
    };

    const syncSnapshot = async (sourceLabel) => {
      if (cancelled || snapshotInFlightRef.current) {
        return;
      }
      snapshotInFlightRef.current = true;
      try {
        const snapshot = await fetchJson(apiBase, snapshotPath);
        if (cancelled) {
          return;
        }
        setHasError(false);
        applySnapshot(snapshot, sourceLabel);
      } catch (error) {
        if (cancelled) {
          return;
        }
        setHasError(true);
        setStatusMessage(`Snapshot sync failed: ${error.message}`);
      } finally {
        snapshotInFlightRef.current = false;
      }
    };

    const scheduleSnapshot = (sourceLabel, immediate = false) => {
      if (cancelled || snapshotTimerRef.current) {
        return;
      }
      const now = Date.now();
      const delay = immediate
        ? 0
        : Math.max(0, nextSnapshotAtRef.current - now);
      snapshotTimerRef.current = window.setTimeout(async () => {
        snapshotTimerRef.current = null;
        nextSnapshotAtRef.current = Date.now() + snapshotThrottleMs;
        await syncSnapshot(sourceLabel);
      }, delay);
    };

    const scheduleReconnect = () => {
      if (cancelled || reconnectTimerRef.current) {
        return;
      }
      const attempt = reconnectAttemptsRef.current;
      const delay = Math.min(
        streamMaxReconnectMs,
        streamInitialReconnectMs * 2 ** attempt,
      );
      reconnectAttemptsRef.current = Math.min(attempt + 1, 12);
      reconnectTimerRef.current = window.setTimeout(() => {
        reconnectTimerRef.current = null;
        connectStream();
      }, delay);
    };

    const connectStream = () => {
      if (cancelled) {
        return;
      }
      cleanupSocket();
      const streamUrl = resolveStreamUrl(apiBase, latestSeqIdRef.current);
      const socket = new WebSocket(streamUrl);
      streamSocketRef.current = socket;

      socket.onopen = () => {
        reconnectAttemptsRef.current = 0;
        setHasError(false);
        scheduleSnapshot('ws-open', true);
      };

      socket.onmessage = (message) => {
        if (cancelled) {
          return;
        }
        let payload;
        try {
          payload = JSON.parse(message.data);
        } catch {
          return;
        }

        if (payload?.event === 'hello') {
          scheduleSnapshot('ws-hello', true);
          return;
        }

        if (typeof payload?.seq_id !== 'number') {
          return;
        }

        const seqId = payload.seq_id;
        if (seqId <= latestSeqIdRef.current) {
          return;
        }

        const hasGap =
          latestSeqIdRef.current > 0 && seqId > latestSeqIdRef.current + 1;
        latestSeqIdRef.current = seqId;
        scheduleSnapshot(hasGap ? 'ws-gap-resync' : 'ws-delta', hasGap);
      };

      socket.onerror = () => {
        socket.close();
      };

      socket.onclose = () => {
        if (cancelled) {
          return;
        }
        scheduleReconnect();
      };
    };

    setStatusMessage(`Connecting stream to ${apiBase} ...`);
    setHasError(false);
    scheduleSnapshot('bootstrap', true);
    connectStream();

    return () => {
      cancelled = true;
      clearReconnectTimer();
      clearSnapshotTimer();
      cleanupSocket();
    };
  }, [
    apiBase,
    maxTransactionHistory,
    snapshotTxLimit,
    transactionRetentionMinutes,
    transactionRetentionMs,
  ]);

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
        setTransactionDetailsByHash((current) => {
          const next = {
            ...current,
            [dialogHash]: detail,
          };
          const hashes = Object.keys(next);
          if (hashes.length <= detailCacheLimit) {
            return next;
          }
          const evictCount = hashes.length - detailCacheLimit;
          for (let index = 0; index < evictCount; index += 1) {
            delete next[hashes[index]];
          }
          return next;
        });
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
        closeDialog();
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [closeDialog, dialogHash]);

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
        row.source_id?.toLowerCase().includes(needle) ||
        String(row.tx_type ?? '').toLowerCase().includes(needle) ||
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
  const selectedOpportunity = useMemo(
    () => opportunityRows.find((row) => opportunityRowKey(row) === selectedOpportunityKey) ?? null,
    [opportunityRows, selectedOpportunityKey],
  );
  const selectedArchiveTx = useMemo(
    () => archiveTxRows.find((row) => row.hash === selectedArchiveTxHash) ?? null,
    [archiveTxRows, selectedArchiveTxHash],
  );
  const selectedArchiveOpp = useMemo(
    () => archiveOppRows.find((row) => opportunityRowKey(row) === selectedArchiveOppKey) ?? null,
    [archiveOppRows, selectedArchiveOppKey],
  );

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
  const detailSender = dialogTransaction?.sender ?? dialogDetail?.sender ?? '-';
  const detailSeenUnixMs = dialogDetail?.first_seen_unix_ms ?? dialogTransaction?.seen_unix_ms;
  const detailSeenAt = formatTime(detailSeenUnixMs);
  const detailSeenRelative = formatRelativeTime(detailSeenUnixMs);

  const currentFrame = replayFrames[timelineIndex] ?? null;
  const latestReplayFrames = useMemo(
    () => replayFrames.slice(-160).reverse(),
    [replayFrames],
  );
  const transactionPageSize = tickerPageSize;
  const maxPagedRows = transactionPageSize * tickerPageLimit;
  const pagedTransactions = useMemo(
    () => filteredTransactions.slice(0, maxPagedRows),
    [filteredTransactions, maxPagedRows],
  );
  const transactionPageCount = Math.max(
    1,
    Math.ceil(pagedTransactions.length / transactionPageSize),
  );
  const normalizedTransactionPage = Math.min(transactionPage, transactionPageCount);
  const transactionPageStart = (normalizedTransactionPage - 1) * transactionPageSize;
  const latestTickerRows = pagedTransactions.slice(
    transactionPageStart,
    transactionPageStart + transactionPageSize,
  );
  const deferredTickerRows = useDeferredValue(latestTickerRows);
  const transactionPageEnd = Math.min(
    pagedTransactions.length,
    transactionPageStart + latestTickerRows.length,
  );
  const paginationPages = useMemo(
    () => paginationWindow(normalizedTransactionPage, transactionPageCount),
    [normalizedTransactionPage, transactionPageCount],
  );
  const archiveNeedle = archiveQuery.trim().toLowerCase();
  const filteredArchiveTxRows = useMemo(() => {
    if (!archiveNeedle) {
      return archiveTxRows;
    }
    return archiveTxRows.filter((row) => (
      row.hash?.toLowerCase().includes(archiveNeedle) ||
      row.sender?.toLowerCase().includes(archiveNeedle) ||
      row.peer?.toLowerCase().includes(archiveNeedle) ||
      row.protocol?.toLowerCase().includes(archiveNeedle) ||
      row.category?.toLowerCase().includes(archiveNeedle) ||
      row.lifecycle_status?.toLowerCase().includes(archiveNeedle) ||
      String(row.nonce ?? '').includes(archiveNeedle)
    ));
  }, [archiveNeedle, archiveTxRows]);
  const filteredArchiveOppRows = useMemo(() => {
    if (!archiveNeedle) {
      return archiveOppRows;
    }
    return archiveOppRows.filter((row) => (
      row.tx_hash?.toLowerCase().includes(archiveNeedle) ||
      row.strategy?.toLowerCase().includes(archiveNeedle) ||
      row.protocol?.toLowerCase().includes(archiveNeedle) ||
      row.category?.toLowerCase().includes(archiveNeedle) ||
      row.status?.toLowerCase().includes(archiveNeedle) ||
      row.reasons?.some((reason) => reason?.toLowerCase().includes(archiveNeedle))
    ));
  }, [archiveNeedle, archiveOppRows]);
  const archiveTxPageCount = Math.max(
    1,
    Math.ceil(filteredArchiveTxRows.length / archiveTxPageSize),
  );
  const normalizedArchiveTxPage = Math.min(archiveTxPage, archiveTxPageCount);
  const archiveTxStart = (normalizedArchiveTxPage - 1) * archiveTxPageSize;
  const archiveTxPageRows = filteredArchiveTxRows.slice(
    archiveTxStart,
    archiveTxStart + archiveTxPageSize,
  );
  const archiveTxPages = useMemo(
    () => paginationWindow(normalizedArchiveTxPage, archiveTxPageCount),
    [archiveTxPageCount, normalizedArchiveTxPage],
  );
  const archiveOppPageCount = Math.max(
    1,
    Math.ceil(filteredArchiveOppRows.length / archiveOppPageSize),
  );
  const normalizedArchiveOppPage = Math.min(archiveOppPage, archiveOppPageCount);
  const archiveOppStart = (normalizedArchiveOppPage - 1) * archiveOppPageSize;
  const archiveOppPageRows = filteredArchiveOppRows.slice(
    archiveOppStart,
    archiveOppStart + archiveOppPageSize,
  );
  const archiveOppPages = useMemo(
    () => paginationWindow(normalizedArchiveOppPage, archiveOppPageCount),
    [archiveOppPageCount, normalizedArchiveOppPage],
  );

  useEffect(() => {
    setTransactionPage((current) => Math.min(current, transactionPageCount));
  }, [transactionPageCount]);
  useEffect(() => {
    setArchiveTxPage((current) => Math.min(current, archiveTxPageCount));
  }, [archiveTxPageCount]);
  useEffect(() => {
    setArchiveOppPage((current) => Math.min(current, archiveOppPageCount));
  }, [archiveOppPageCount]);
  useEffect(() => {
    setArchiveTxPage(1);
    setArchiveOppPage(1);
  }, [archiveQuery]);
  const {
    totalSignalVolume,
    totalTxCount,
    lowRiskCount,
    mediumRiskCount,
    highRiskCount,
    successRate,
  } = marketStats;
  const featureTrendCounts = useMemo(() => {
    const values = featureSummaryRows.slice(0, 10).map((row) => row.count);
    return values.length ? values.reverse() : [0, 0, 0];
  }, [featureSummaryRows]);
  const featureTrendPath = useMemo(
    () => sparklinePath(featureTrendCounts, 100, 34),
    [featureTrendCounts],
  );
  const { topMixRows, mixTotal } = useMemo(() => {
    const rows = featureSummaryRows.slice(0, 4);
    const total = rows.reduce((sum, row) => sum + row.count, 0);
    return { topMixRows: rows, mixTotal: total };
  }, [featureSummaryRows]);
  const onShowRadar = useCallback(() => {
    setActiveScreen('radar');
  }, []);
  const onShowOpps = useCallback(() => {
    setActiveScreen('opps');
  }, []);
  const onShowReplay = useCallback(() => {
    setActiveScreen('replay');
  }, []);
  const onSearchChange = useCallback((event) => {
    setQuery(event.target.value);
  }, []);
  const onTickerFollowClick = useCallback(() => {
    setFollowLatest(true);
    setTransactionPage(1);
    if (replayFrames.length) {
      setTimelineIndex(replayFrames.length - 1);
    }
  }, [replayFrames]);
  const onTransactionPagePrev = useCallback(() => {
    setFollowLatest(false);
    setTransactionPage((current) => Math.max(1, current - 1));
  }, []);
  const onTransactionPageNext = useCallback(() => {
    setFollowLatest(false);
    setTransactionPage((current) => Math.min(transactionPageCount, current + 1));
  }, [transactionPageCount]);
  const onTransactionPaginationClick = useCallback((event) => {
    const node = event.target;
    if (!(node instanceof Element)) {
      return;
    }
    const button = node.closest('button[data-page]');
    const pageText = button?.getAttribute('data-page');
    const page = Number(pageText);
    if (!Number.isFinite(page)) {
      return;
    }
    setTransactionPage(Math.max(1, Math.min(transactionPageCount, page)));
    if (page !== 1) {
      setFollowLatest(false);
    }
  }, [transactionPageCount]);
  const onArchiveQueryChange = useCallback((event) => {
    setArchiveQuery(event.target.value);
  }, []);
  const onArchiveTxPagePrev = useCallback(() => {
    setArchiveTxPage((current) => Math.max(1, current - 1));
  }, []);
  const onArchiveTxPageNext = useCallback(() => {
    setArchiveTxPage((current) => Math.min(archiveTxPageCount, current + 1));
  }, [archiveTxPageCount]);
  const onArchiveTxPaginationClick = useCallback((event) => {
    const node = event.target;
    if (!(node instanceof Element)) {
      return;
    }
    const button = node.closest('button[data-archive-tx-page]');
    const pageText = button?.getAttribute('data-archive-tx-page');
    const page = Number(pageText);
    if (!Number.isFinite(page)) {
      return;
    }
    setArchiveTxPage(Math.max(1, Math.min(archiveTxPageCount, page)));
  }, [archiveTxPageCount]);
  const onArchiveOppPagePrev = useCallback(() => {
    setArchiveOppPage((current) => Math.max(1, current - 1));
  }, []);
  const onArchiveOppPageNext = useCallback(() => {
    setArchiveOppPage((current) => Math.min(archiveOppPageCount, current + 1));
  }, [archiveOppPageCount]);
  const onArchiveOppPaginationClick = useCallback((event) => {
    const node = event.target;
    if (!(node instanceof Element)) {
      return;
    }
    const button = node.closest('button[data-archive-opp-page]');
    const pageText = button?.getAttribute('data-archive-opp-page');
    const page = Number(pageText);
    if (!Number.isFinite(page)) {
      return;
    }
    setArchiveOppPage(Math.max(1, Math.min(archiveOppPageCount, page)));
  }, [archiveOppPageCount]);
  const onReplayTimelineChange = useCallback((event) => {
    setTimelineIndex(Number(event.target.value));
    setFollowLatest(false);
  }, []);
  const onReplayFollowClick = useCallback(() => {
    setFollowLatest(true);
    if (replayFrames.length) {
      setTimelineIndex(replayFrames.length - 1);
    }
  }, [replayFrames]);
  const editionDate = new Date().toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
    year: 'numeric',
  });

  return (
    <div className="news-body min-h-screen text-zinc-900">
      <div className="newspaper-shell flex h-screen w-screen max-w-none flex-col overflow-hidden">
        <header className="news-masthead px-5 pb-3 pt-4">
          <div className="news-mono mb-3 flex items-center justify-between border-b border-zinc-900 pb-2 text-[11px] uppercase tracking-[0.15em]">
            <span>Vol. 03 · Prototype Desk</span>
            <span>{editionDate}</span>
            <span>Global Edition</span>
          </div>
          <div className="relative mb-4 flex items-center justify-center border-b border-zinc-900 pb-3">
            <h1 className="news-headline text-center text-4xl font-extrabold uppercase leading-none tracking-tight md:text-6xl">
              Mempulse
            </h1>
            <div className="news-mono absolute right-0 top-1/2 hidden -translate-y-1/2 border border-zinc-900 bg-[#f7f1e6] px-2 py-1 text-[10px] uppercase tracking-[0.12em] lg:block">
              Live Wire
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 border-y-2 border-zinc-900 py-2">
            <button
              type="button"
              onClick={onShowRadar}
              className={cn(
                'news-tab news-mono cursor-pointer px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                activeScreen === 'radar' ? 'news-tab-active' : '',
              )}
            >
              Front Page
            </button>
            <button
              type="button"
              onClick={onShowOpps}
              className={cn(
                'news-tab news-mono cursor-pointer px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                activeScreen === 'opps' ? 'news-tab-active' : '',
              )}
            >
              Opportunity Desk
            </button>
            <button
              type="button"
              onClick={onShowReplay}
              className={cn(
                'news-tab news-mono cursor-pointer px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                activeScreen === 'replay' ? 'news-tab-active' : '',
              )}
            >
              Archives
            </button>
            <div className="news-mono ml-auto text-[11px] uppercase tracking-[0.12em] text-zinc-700">
              {statusMessage}
            </div>
          </div>
        </header>

        {activeScreen === 'radar' ? (
          <div className="grid min-h-0 flex-1 grid-cols-1 gap-6 overflow-hidden bg-[#f7f1e6] p-4 lg:grid-cols-[minmax(0,1fr)_minmax(360px,420px)] xl:grid-cols-[minmax(0,1fr)_460px]">
            <main className="flex min-h-0 flex-col overflow-hidden lg:border-r-2 lg:border-zinc-900 lg:pr-6">
              <div className="mb-4 text-center">
                <h2 className="news-headline text-3xl font-bold italic md:text-4xl">
                  Real-Time Transaction Monitor
                </h2>
                <div className="news-mono mt-1 flex items-center justify-center gap-2 text-[11px] uppercase tracking-[0.12em] text-zinc-700">
                  <span className="inline-block size-2 animate-pulse rounded-full bg-zinc-900" />
                  <span>Live Wire Service • Updates Continuously</span>
                </div>
              </div>

              <div className="mb-1 flex flex-wrap items-center justify-between gap-2 border-b-4 border-zinc-900 pb-2">
                <div className="news-section-title text-xl font-bold uppercase">Latest Ticker</div>
                <div className="flex gap-2">
                  <button
                    type="button"
                    className="news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors"
                  >
                    Filter
                  </button>
                  <button
                    type="button"
                    onClick={onTickerFollowClick}
                    className="news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors"
                  >
                    Follow
                  </button>
                </div>
              </div>

              <div className="mb-2 flex items-center gap-2">
                <input
                  type="search"
                  value={query}
                  onChange={onSearchChange}
                  placeholder="Search hash, sender, source, protocol, category"
                  className="news-mono w-full border border-zinc-900 bg-[#fffdf7] px-3 py-2 text-sm outline-none transition-colors focus:bg-white"
                />
              </div>

              <div className={cn('news-mono mb-3 text-[10px] uppercase tracking-[0.12em]', hasError ? 'text-rose-700' : 'text-zinc-700')}>
                {statusMessage}
              </div>

              <div className="news-list-shell min-h-0 flex flex-1 flex-col overflow-hidden">
                <div
                  className="news-list-scroll h-full border-b-2 border-zinc-900"
                  onClick={onTickerListClick}
                >
                  <table className="news-tx-table news-mono min-w-[1248px] w-full table-fixed border-collapse">
                    <thead className="sticky top-0 z-10 border-b border-zinc-900 bg-zinc-900 text-[#f7f1e6]">
                      <tr className="text-[13px] font-bold uppercase tracking-[0.1em]">
                        <th scope="col" className="w-[170px] px-2 py-2 text-left">Timestamp</th>
                        <th scope="col" className="w-[170px] px-2 py-2 text-left">Ref. ID</th>
                        <th scope="col" className="w-[170px] px-2 py-2 text-left">Sender</th>
                        <th scope="col" className="w-[84px] px-2 py-2 text-left">Source</th>
                        <th scope="col" className="w-[66px] px-2 py-2 text-left">Type</th>
                        <th scope="col" className="w-[72px] px-2 py-2 text-left">Nonce</th>
                        <th scope="col" className="w-[102px] px-2 py-2 text-left">Protocol</th>
                        <th scope="col" className="w-[100px] px-2 py-2 text-left">Category</th>
                        <th scope="col" className="w-[76px] px-2 py-2 text-right">Amt.</th>
                        <th scope="col" className="w-[104px] px-2 py-2 text-left">Status</th>
                        <th scope="col" className="w-[74px] px-2 py-2 text-left">Risk</th>
                        <th scope="col" className="w-[36px] px-2 py-2 text-center">Op.</th>
                      </tr>
                    </thead>
                    <tbody>
                      {deferredTickerRows.map((row) => {
                        const feature = featureByHash.get(row.hash);
                        const risk = classifyRisk(feature);
                        const status = statusForRow(feature);
                        const isActive = row.hash === selectedHash;
                        const amountValue = feature
                          ? (feature.urgency_score / 10).toFixed(1)
                          : '--.--';
                        const protocolValue = feature?.protocol ?? '-';
                        const categoryValue = feature?.category ?? '-';
                        const senderValue = row?.sender ?? '-';
                        const sourceValue = row?.source_id ?? '-';
                        const nonceValue = row?.nonce ?? '-';
                        const txTypeValue = row?.tx_type ?? '-';

                        return (
                          <tr
                            key={row.hash}
                            data-tx-hash={row.hash}
                            className={cn(
                              'news-tx-row cursor-pointer border-b border-dashed border-zinc-900 text-[13px]',
                              isActive
                                ? 'bg-zinc-900 text-[#f7f1e6]'
                                : status === 'Flagged'
                                  ? 'bg-zinc-900 text-[#f7f1e6]'
                                  : 'bg-[#fffdf7] text-zinc-800 hover:bg-black/5',
                            )}
                          >
                            <td className="px-2 py-2 align-middle whitespace-nowrap">{formatTickerTime(row.seen_unix_ms)}</td>
                            <td className="px-2 py-2 align-middle">
                              <span className="block truncate" title={row.hash}>
                                {shortHex(row.hash, 18, 8)}
                              </span>
                            </td>
                            <td className="px-2 py-2 align-middle">
                              <span className="block truncate" title={senderValue}>
                                {shortHex(senderValue, 10, 8)}
                              </span>
                            </td>
                            <td className="px-2 py-2 align-middle">
                              <span className="block truncate" title={sourceValue}>
                                {sourceValue}
                              </span>
                            </td>
                            <td className="px-2 py-2 align-middle">{txTypeValue}</td>
                            <td className="px-2 py-2 align-middle">{nonceValue}</td>
                            <td className="px-2 py-2 align-middle">
                              <span className="block truncate" title={protocolValue}>
                                {protocolValue}
                              </span>
                            </td>
                            <td className="px-2 py-2 align-middle">
                              <span className="block truncate" title={categoryValue}>
                                {categoryValue}
                              </span>
                            </td>
                            <td className="px-2 py-2 text-right align-middle">{amountValue}</td>
                            <td className="px-2 py-2 align-middle">
                              <span
                                className={cn(
                                  'border px-1 text-[12px] font-bold uppercase tracking-[0.08em]',
                                  statusBadgeClass(status, isActive || status === 'Flagged'),
                                )}
                              >
                                {status}
                              </span>
                            </td>
                            <td className="px-2 py-2 align-middle">
                              <span
                                className={cn(
                                  'border px-1 text-[12px] font-bold uppercase tracking-[0.08em]',
                                  riskBadgeClass(risk.label, isActive || status === 'Flagged'),
                                )}
                              >
                                {risk.label}
                              </span>
                            </td>
                            <td className={cn('px-2 py-2 text-center align-middle font-bold', isActive ? 'text-[#f7f1e6]' : 'text-zinc-500')}>
                              •••
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </div>

              {filteredTransactions.length > 0 ? (
                <div className="mt-3 border-t-2 border-zinc-900 pt-2">
                  <div className="news-mono mb-2 text-center text-[10px] uppercase tracking-[0.14em] text-zinc-700">
                    Showing {transactionPageStart + 1}-{transactionPageEnd} of {pagedTransactions.length} rows · page {normalizedTransactionPage}/{transactionPageCount}
                  </div>
                  <div className="flex items-center justify-center gap-1.5" onClick={onTransactionPaginationClick}>
                    <button
                      type="button"
                      onClick={onTransactionPagePrev}
                      disabled={normalizedTransactionPage <= 1}
                      className={cn(
                        'news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors disabled:cursor-not-allowed disabled:opacity-40',
                      )}
                    >
                      Prev
                    </button>
                    {paginationPages.map((page) => (
                      <button
                        type="button"
                        key={page}
                        data-page={page}
                        className={cn(
                          'news-tab news-mono cursor-pointer px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors',
                          page === normalizedTransactionPage ? 'news-tab-active' : '',
                        )}
                      >
                        {page}
                      </button>
                    ))}
                    <button
                      type="button"
                      onClick={onTransactionPageNext}
                      disabled={normalizedTransactionPage >= transactionPageCount}
                      className={cn(
                        'news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] transition-colors disabled:cursor-not-allowed disabled:opacity-40',
                      )}
                    >
                      Next
                    </button>
                  </div>
                </div>
              ) : null}

              {filteredTransactions.length === 0 ? (
                <div className="news-dotted-box mt-4 bg-[#fffdf7] p-8 text-center text-sm text-zinc-700">
                  No rows match your search.
                </div>
              ) : null}
            </main>

            <aside className="grid min-h-0 min-w-0 grid-rows-[30fr_45fr_25fr] gap-3 overflow-hidden pr-0.5">
              <div className="news-dotted-box relative flex h-full min-h-0 flex-col overflow-hidden bg-transparent p-4">
                <div className="absolute -left-1 -top-1 h-3 w-3 border-l-2 border-t-2 border-zinc-900" />
                <div className="absolute -right-1 -top-1 h-3 w-3 border-r-2 border-t-2 border-zinc-900" />
                <div className="absolute -bottom-1 -left-1 h-3 w-3 border-b-2 border-l-2 border-zinc-900" />
                <div className="absolute -bottom-1 -right-1 h-3 w-3 border-b-2 border-r-2 border-zinc-900" />
                <h3 className="news-headline border-b-2 border-zinc-900 pb-2 text-center text-xl font-black uppercase tracking-tight xl:text-2xl">
                  Market Statistics
                </h3>

                <div className="mt-3 flex min-h-0 flex-1 flex-col justify-between gap-3 overflow-y-auto pr-1 [scrollbar-gutter:stable]">
                  <div className="text-center">
                    <p className="news-kicker mb-1">Total Signal Volume</p>
                    <p className="news-headline text-3xl font-bold">
                      <RollingInt value={totalSignalVolume} durationMs={650} />
                    </p>
                    <p className="news-mono mt-1 inline-block border-t border-zinc-900 px-2 pt-1 text-[11px] uppercase tracking-[0.12em]">
                      <RollingPercent value={successRate} durationMs={460} suffix="% stable stream" />
                    </p>
                  </div>

                  <div className="news-double-divider" />

                  <div className="grid grid-cols-2 gap-2">
                    <div className="text-center">
                      <p className="news-kicker mb-1">Tx Count</p>
                      <p className="news-headline text-2xl font-bold">
                        <RollingInt value={totalTxCount} durationMs={500} />
                      </p>
                      <p className="news-mono mt-1 text-[10px] uppercase tracking-[0.12em]">
                        +{replayFrames.length} replay frames
                      </p>
                    </div>
                    <div className="border-l border-dashed border-zinc-900 pl-2 text-center">
                      <p className="news-kicker mb-1">Success Rate</p>
                      <p className="news-headline text-2xl font-bold">
                        <RollingPercent value={successRate} durationMs={460} />
                      </p>
                      <p className="news-mono mt-1 text-[10px] uppercase tracking-[0.12em]">
                        {highRiskCount} high risk rows
                      </p>
                    </div>
                  </div>
                </div>
              </div>

              <div className="news-dotted-box min-h-0 flex h-full flex-col overflow-y-auto bg-transparent p-3 [scrollbar-gutter:stable]">
                <h3 className="news-headline mb-2 border-b border-zinc-900 pb-1 text-lg font-bold uppercase xl:text-xl">
                  Feature Engine Report
                </h3>

                <div className="relative mb-2 flex min-h-0 flex-[0_0_48%] flex-col border border-zinc-900 bg-[#fffdf7] p-2">
                  <div className="news-mono absolute right-0 top-0 bg-zinc-900 px-1 text-[10px] uppercase tracking-[0.12em] text-[#f7f1e6]">
                    Fig. 1A
                  </div>
                  <div className="w-full min-h-0 flex-1 pt-2">
                    <svg viewBox="0 0 100 40" className="h-full w-full overflow-visible" preserveAspectRatio="none">
                      <line x1="0" y1="10" x2="100" y2="10" stroke="#c4c4c4" strokeWidth="0.5" strokeDasharray="2 1" />
                      <line x1="0" y1="20" x2="100" y2="20" stroke="#c4c4c4" strokeWidth="0.5" strokeDasharray="2 1" />
                      <line x1="0" y1="30" x2="100" y2="30" stroke="#c4c4c4" strokeWidth="0.5" strokeDasharray="2 1" />
                      <path d={featureTrendPath} fill="none" stroke="#111827" strokeWidth="1.6" vectorEffect="non-scaling-stroke" />
                    </svg>
                  </div>
                  <p className="news-mono mt-2 text-center text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                    Fig 1. Feature mix pressure trend.
                  </p>
                </div>

                <ul className="space-y-1 text-[13px]">
                  <li className="flex items-center justify-between gap-2 border-b border-dashed border-zinc-500 pb-1">
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'news-mono border px-1 text-[10px] font-bold uppercase tracking-[0.1em]',
                          riskBadgeClass('Low'),
                        )}
                      >
                        Low
                      </span>
                      <span className="text-[13px] leading-tight">Risk (velocity normal)</span>
                    </span>
                    <span
                      className={cn(
                        'news-mono border px-1 text-[11px] font-bold uppercase tracking-[0.1em]',
                        riskBadgeClass('Low'),
                      )}
                    >
                      {lowRiskCount}
                    </span>
                  </li>
                  <li className="flex items-center justify-between gap-2 border-b border-dashed border-zinc-500 pb-1">
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'news-mono border px-1 text-[10px] font-bold uppercase tracking-[0.1em]',
                          riskBadgeClass('Medium'),
                        )}
                      >
                        Medium
                      </span>
                      <span className="text-[13px] leading-tight">Risk (pattern drift)</span>
                    </span>
                    <span
                      className={cn(
                        'news-mono border px-1 text-[11px] font-bold uppercase tracking-[0.1em]',
                        riskBadgeClass('Medium'),
                      )}
                    >
                      {mediumRiskCount}
                    </span>
                  </li>
                  <li className="flex items-center justify-between gap-2 pb-1 font-bold">
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'news-mono border px-1 text-[10px] font-bold uppercase tracking-[0.1em]',
                          riskBadgeClass('High'),
                        )}
                      >
                        High
                      </span>
                      <span className="text-[13px] leading-tight">Risk (opportunity spike)</span>
                    </span>
                    <span
                      className={cn(
                        'news-mono border px-1 text-[11px] font-bold uppercase tracking-[0.1em]',
                        riskBadgeClass('High'),
                      )}
                    >
                      {highRiskCount}
                    </span>
                  </li>
                </ul>

                <div className="news-mono mt-2 border-t border-zinc-900 pt-1 text-[9px] uppercase tracking-[0.1em] text-zinc-700">
                  {topMixRows.map((row) => {
                    const pct = mixTotal ? Math.round((row.count / mixTotal) * 100) : 0;
                    return (
                      <div key={`${row.protocol}-${row.category}`} className="flex justify-between py-0.5">
                        <span>{row.protocol} / {row.category}</span>
                        <span>{pct}%</span>
                      </div>
                    );
                  })}
                </div>
              </div>

              <div className="border-2 border-zinc-900 bg-transparent p-3 h-full min-h-0 flex flex-col overflow-y-auto [scrollbar-gutter:stable]">
                <h4 className="news-headline mb-1 text-center text-base font-bold uppercase">
                  Selected Brief
                </h4>
                <div className="news-mono text-xs uppercase tracking-[0.1em]">
                  <div className="mb-1.5">
                    <span className="block text-zinc-500">Tx</span>
                    <span className="block truncate">{selectedTransaction ? shortHex(selectedTransaction.hash, 18, 8) : '-'}</span>
                  </div>
                  <div className="mb-1.5">
                    <span className="block text-zinc-500">Protocol</span>
                    <span className="block">{selectedFeature?.protocol ?? '-'}</span>
                  </div>
                  <div className="mb-1.5">
                    <span className="block text-zinc-500">Category</span>
                    <span className="block">{selectedFeature?.category ?? '-'}</span>
                  </div>
                  <div className="grid grid-cols-2 gap-1.5 border-t border-zinc-900 pt-1.5">
                    <div>
                      <span className="block text-zinc-500">Mev</span>
                      <span className="text-base font-bold">{selectedFeature?.mev_score ?? '-'}</span>
                    </div>
                    <div>
                      <span className="block text-zinc-500">Urgency</span>
                      <span className="text-base font-bold">{selectedFeature?.urgency_score ?? '-'}</span>
                    </div>
                  </div>
                  <div className="mt-1.5 border-t border-zinc-900 pt-1.5 text-[10px]">
                    {selectedRecent
                      ? `Source ${selectedRecent.source_id} observed type ${selectedRecent.tx_type}.`
                      : 'Select a row to inspect metadata.'}
                  </div>
                </div>
              </div>
            </aside>
          </div>
        ) : null}

        {activeScreen === 'opps' ? (
          <div className="grid min-h-0 flex-1 grid-cols-1 bg-[#f7f1e6] lg:grid-cols-[420px_minmax(500px,1fr)]">
            <section className="min-h-0 overflow-auto border-b border-zinc-900 p-4 lg:border-b-0 lg:border-r">
              <div className="news-kicker mb-3">
                Opportunity Pipeline
              </div>
              <div className="space-y-2" onClick={onOpportunityListClick}>
                {opportunityRows.map((opportunity) => {
                  const rowKey = opportunityRowKey(opportunity);
                  const isActive = selectedOpportunityKey === rowKey;
                  const tone = opportunityCandidateTone(opportunity, isActive);
                  return (
                    <button
                      type="button"
                      key={rowKey}
                      data-opportunity-key={rowKey}
                      className={cn(
                        'w-full cursor-pointer border p-3 text-left transition-colors',
                        tone.container,
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="news-headline text-sm font-semibold">{opportunity.strategy}</div>
                        <div className={cn('news-mono text-[11px] uppercase tracking-[0.1em]', tone.subtle)}>
                          score {opportunity.score}
                        </div>
                      </div>
                      <div className={cn('news-mono mt-1 text-[11px] uppercase tracking-[0.1em]', tone.subtle)}>
                        {opportunity.protocol} · {opportunity.category}
                      </div>
                      <div className={cn('news-mono mt-1 text-[11px] uppercase tracking-[0.1em]', tone.subtle)}>
                        tx {shortHex(opportunity.tx_hash, 14, 10)} · {formatRelativeTime(opportunity.detected_unix_ms)}
                      </div>
                    </button>
                  );
                })}
              </div>
              {opportunityRows.length === 0 ? (
                <div className="news-dotted-box mt-4 bg-[#fffdf7] p-6 text-sm text-zinc-700">
                  No opportunities available.
                </div>
              ) : null}
            </section>

            <section className="min-h-0 overflow-auto p-4">
              <div className="news-card p-4">
                <div className="news-kicker">
                  Selected Opportunity
                </div>
                {selectedOpportunity ? (
                  <div className="mt-3 space-y-3 text-sm">
                    <div>
                      <div className="news-kicker">Transaction</div>
                      <div className="news-mono text-[13px]">{selectedOpportunity.tx_hash}</div>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="news-kicker">Strategy</div>
                        <div>{selectedOpportunity.strategy}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Score</div>
                        <div>{selectedOpportunity.score}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Protocol</div>
                        <div>{selectedOpportunity.protocol}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Category</div>
                        <div>{selectedOpportunity.category}</div>
                      </div>
                    </div>
                    <div className="border border-zinc-900 bg-[#fffdf7] p-3">
                      <div className="news-kicker">
                        Rule Versions
                      </div>
                      <div className="news-mono mt-2 text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        feature-engine: {selectedOpportunity.feature_engine_version}
                      </div>
                      <div className="news-mono text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        scorer: {selectedOpportunity.scorer_version}
                      </div>
                      <div className="news-mono text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        strategy: {selectedOpportunity.strategy_version}
                      </div>
                    </div>
                    <div>
                      <div className="news-kicker">Reasons</div>
                      <ul className="news-mono mt-1 space-y-1 text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        {selectedOpportunity.reasons?.length
                          ? selectedOpportunity.reasons.map((reason, index) => (
                              <li key={`${reason}-${index}`} className="border border-zinc-900/40 bg-[#fffdf7] px-2 py-1">
                                {reason}
                              </li>
                            ))
                          : [<li key="none" className="border border-zinc-900/40 bg-[#fffdf7] px-2 py-1">No reasons provided.</li>]}
                      </ul>
                    </div>
                  </div>
                ) : (
                  <div className="mt-3 text-sm text-zinc-700">Select an opportunity from the left pane.</div>
                )}
              </div>
            </section>
          </div>
        ) : null}

        {activeScreen === 'replay' ? (
          <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 bg-[#f7f1e6] p-4 lg:grid-cols-[minmax(0,1.25fr)_minmax(360px,1fr)]">
            <section className="flex h-full min-h-0 flex-col overflow-hidden border border-zinc-900 bg-[#fffdf7] p-4">
              <div className="flex flex-wrap items-end justify-between gap-3 border-b-2 border-zinc-900 pb-2">
                <div>
                  <div className="news-kicker">Archive Desk</div>
                  <h2 className="news-headline text-2xl font-bold">Historical Records</h2>
                </div>
                <button
                  type="button"
                  onClick={refreshArchives}
                  className={cn(
                    'news-tab news-mono cursor-pointer px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                    archiveLoading ? 'opacity-60' : '',
                  )}
                  disabled={archiveLoading}
                >
                  {archiveLoading ? 'Syncing' : 'Refresh'}
                </button>
              </div>

              <div className="mt-3 flex flex-wrap items-center gap-2">
                <input
                  value={archiveQuery}
                  onChange={onArchiveQueryChange}
                  placeholder="Search hash, sender, protocol, category, status"
                  className="news-mono min-w-[18rem] flex-1 border border-zinc-900 bg-[#fffdf7] px-3 py-2 text-[12px] uppercase tracking-[0.08em] outline-none focus:bg-white"
                />
                <div className="news-mono text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                  tx:{archiveTxRows.length} · opps:{archiveOppRows.length}
                </div>
              </div>

              {archiveError ? (
                <div className="news-mono mt-2 border border-rose-900 bg-rose-100/70 px-3 py-2 text-[11px] uppercase tracking-[0.1em] text-rose-800">
                  {archiveError}
                </div>
              ) : null}

              <div className="mt-3 grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_minmax(0,1fr)] gap-3 overflow-hidden">
                <div className="news-card min-h-0 flex flex-col p-3">
                  <div className="mb-2 flex items-center justify-between border-b border-zinc-900 pb-1">
                    <div className="news-kicker">Historical Transactions</div>
                    <div className="news-mono text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                      {filteredArchiveTxRows.length} rows
                    </div>
                  </div>
                  <div className="news-list-scroll min-h-0 flex-1 space-y-1" onClick={onArchiveTxListClick}>
                    {archiveTxPageRows.map((row) => {
                      const isActive = selectedArchiveTxHash === row.hash;
                      const lifecycle = row.lifecycle_status ?? 'pending';
                      return (
                        <button
                          key={row.hash}
                          type="button"
                          data-archive-tx-hash={row.hash}
                          className={cn(
                            'w-full cursor-pointer border px-3 py-2 text-left transition-colors',
                            isActive
                              ? 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]'
                              : 'border-zinc-900 bg-[#fffdf7] text-zinc-900 hover:bg-zinc-100/60',
                          )}
                        >
                          <div className="news-mono text-[11px] uppercase tracking-[0.1em]">
                            {shortHex(row.hash, 16, 10)}
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', isActive ? 'text-zinc-300' : 'text-zinc-700')}>
                            {formatTime(row.first_seen_unix_ms)} · {row.protocol ?? 'unknown'} / {row.category ?? 'pending'}
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', isActive ? 'text-zinc-300' : 'text-zinc-700')}>
                            {lifecycle} · mev {row.mev_score ?? '-'} · urgency {row.urgency_score ?? '-'}
                          </div>
                        </button>
                      );
                    })}
                    {archiveTxPageRows.length === 0 ? (
                      <div className="news-mono border border-zinc-900 bg-[#fffdf7] px-3 py-4 text-center text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        No archived transactions found.
                      </div>
                    ) : null}
                  </div>
                  <div className="mt-2 flex items-center justify-between border-t border-zinc-900 pt-2">
                    <button
                      type="button"
                      onClick={onArchiveTxPagePrev}
                      disabled={normalizedArchiveTxPage <= 1}
                      className="news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      Prev
                    </button>
                    <div className="flex items-center gap-1" onClick={onArchiveTxPaginationClick}>
                      {archiveTxPages.map((page) => (
                        <button
                          key={`archive-tx-page-${page}`}
                          type="button"
                          data-archive-tx-page={page}
                          className={cn(
                            'news-tab news-mono cursor-pointer px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                            page === normalizedArchiveTxPage ? 'news-tab-active' : '',
                          )}
                        >
                          {page}
                        </button>
                      ))}
                    </div>
                    <button
                      type="button"
                      onClick={onArchiveTxPageNext}
                      disabled={normalizedArchiveTxPage >= archiveTxPageCount}
                      className="news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      Next
                    </button>
                  </div>
                </div>

                <div className="news-card min-h-0 flex flex-col p-3">
                  <div className="mb-2 flex items-center justify-between border-b border-zinc-900 pb-1">
                    <div className="news-kicker">Historical Opportunities</div>
                    <div className="news-mono text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                      {filteredArchiveOppRows.length} rows
                    </div>
                  </div>
                  <div className="news-list-scroll min-h-0 flex-1 space-y-1" onClick={onArchiveOppListClick}>
                    {archiveOppPageRows.map((row) => {
                      const rowKey = opportunityRowKey(row);
                      const isActive = selectedArchiveOppKey === rowKey;
                      const tone = opportunityCandidateTone(row, isActive);
                      return (
                        <button
                          key={rowKey}
                          type="button"
                          data-archive-opp-key={rowKey}
                          className={cn('w-full cursor-pointer border px-3 py-2 text-left transition-colors', tone.container)}
                        >
                          <div className="flex items-center justify-between gap-2">
                            <div className="news-headline text-sm font-semibold">{row.strategy}</div>
                            <div className={cn('news-mono text-[10px] uppercase tracking-[0.1em]', tone.subtle)}>
                              score {row.score}
                            </div>
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', tone.subtle)}>
                            {row.protocol} · {row.category} · {row.status}
                          </div>
                          <div className={cn('news-mono mt-1 text-[10px] uppercase tracking-[0.1em]', tone.subtle)}>
                            tx {shortHex(row.tx_hash, 14, 10)} · {formatRelativeTime(row.detected_unix_ms)}
                          </div>
                        </button>
                      );
                    })}
                    {archiveOppPageRows.length === 0 ? (
                      <div className="news-mono border border-zinc-900 bg-[#fffdf7] px-3 py-4 text-center text-[11px] uppercase tracking-[0.1em] text-zinc-700">
                        No archived opportunities found.
                      </div>
                    ) : null}
                  </div>
                  <div className="mt-2 flex items-center justify-between border-t border-zinc-900 pt-2">
                    <button
                      type="button"
                      onClick={onArchiveOppPagePrev}
                      disabled={normalizedArchiveOppPage <= 1}
                      className="news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      Prev
                    </button>
                    <div className="flex items-center gap-1" onClick={onArchiveOppPaginationClick}>
                      {archiveOppPages.map((page) => (
                        <button
                          key={`archive-opp-page-${page}`}
                          type="button"
                          data-archive-opp-page={page}
                          className={cn(
                            'news-tab news-mono cursor-pointer px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.12em]',
                            page === normalizedArchiveOppPage ? 'news-tab-active' : '',
                          )}
                        >
                          {page}
                        </button>
                      ))}
                    </div>
                    <button
                      type="button"
                      onClick={onArchiveOppPageNext}
                      disabled={normalizedArchiveOppPage >= archiveOppPageCount}
                      className="news-tab news-mono cursor-pointer px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      Next
                    </button>
                  </div>
                </div>
              </div>
            </section>

            <section className="min-h-0 overflow-auto space-y-3">
              <div className="news-card p-4">
                <div className="news-kicker">Selected Transaction Record</div>
                {selectedArchiveTx ? (
                  <div className="mt-3 space-y-3 text-sm">
                    <div>
                      <div className="news-kicker">Hash</div>
                      <div className="news-mono text-[13px]">{selectedArchiveTx.hash}</div>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="news-kicker">Seen At</div>
                        <div>{formatTime(selectedArchiveTx.first_seen_unix_ms)}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Status</div>
                        <div>{selectedArchiveTx.lifecycle_status ?? 'pending'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Protocol</div>
                        <div>{selectedArchiveTx.protocol ?? 'unknown'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Category</div>
                        <div>{selectedArchiveTx.category ?? 'pending'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Sender</div>
                        <div className="news-mono text-[12px]">{selectedArchiveTx.sender ?? '-'}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Peer</div>
                        <div>{selectedArchiveTx.peer ?? '-'}</div>
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={() => openTransactionByHash(selectedArchiveTx.hash)}
                      className="news-tab news-mono cursor-pointer px-3 py-1.5 text-[11px] font-bold uppercase tracking-[0.12em]"
                    >
                      Inspect Tx Detail
                    </button>
                  </div>
                ) : (
                  <div className="mt-3 text-sm text-zinc-700">Select an archived transaction.</div>
                )}
              </div>

              <div className="news-card p-4">
                <div className="news-kicker">Selected Opportunity Record</div>
                {selectedArchiveOpp ? (
                  <div className="mt-3 space-y-3 text-sm">
                    <div>
                      <div className="news-kicker">Transaction</div>
                      <div className="news-mono text-[13px]">{selectedArchiveOpp.tx_hash}</div>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="news-kicker">Strategy</div>
                        <div>{selectedArchiveOpp.strategy}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Score</div>
                        <div>{selectedArchiveOpp.score}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Protocol</div>
                        <div>{selectedArchiveOpp.protocol}</div>
                      </div>
                      <div>
                        <div className="news-kicker">Category</div>
                        <div>{selectedArchiveOpp.category}</div>
                      </div>
                    </div>
                    <div>
                      <div className="news-kicker">Reasons</div>
                      <ul className="news-mono mt-1 space-y-1 text-[10px] uppercase tracking-[0.1em] text-zinc-700">
                        {(selectedArchiveOpp.reasons?.length
                          ? selectedArchiveOpp.reasons
                          : ['No reasons provided.']).map((reason, index) => (
                          <li key={`${reason}-${index}`} className="border border-zinc-900/40 bg-[#fffdf7] px-2 py-1">
                            {reason}
                          </li>
                        ))}
                      </ul>
                    </div>
                  </div>
                ) : (
                  <div className="mt-3 text-sm text-zinc-700">Select an archived opportunity.</div>
                )}
              </div>
            </section>
          </div>
        ) : null}
      </div>

      {dialogHash ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
          onClick={closeDialog}
        >
          <div
            className="news-dialog-enter relative max-h-[92vh] min-h-[32rem] w-full max-w-5xl overflow-auto border-2 border-zinc-900 bg-[#f7f1e6] p-6 text-[18px] shadow-2xl [scrollbar-gutter:stable]"
            aria-busy={dialogLoading}
            onClick={(event) => event.stopPropagation()}
          >
            {dialogLoading ? (
              <div className="absolute inset-0 z-20 flex items-center justify-center bg-[#f7f1e6]/55 backdrop-blur-[2px]">
                <div className="news-mono border border-zinc-900 bg-[#fffdf7] px-4 py-3 text-[16px] uppercase tracking-[0.08em] text-zinc-700 shadow-sm">
                  Loading on-demand transaction detail...
                </div>
              </div>
            ) : null}

            <div className="news-detail-masthead">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="news-kicker">
                    Transaction Detail
                  </div>
                  <div className="news-mono mt-1 break-all text-[18px] leading-relaxed text-zinc-900">{dialogHash}</div>
                </div>
                <button
                  type="button"
                  onClick={closeDialog}
                  aria-label="Close transaction detail"
                  className="news-icon-button"
                >
                  <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden="true">
                    <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="square" />
                  </svg>
                </button>
              </div>
              <div className="news-mono mt-2 flex flex-wrap items-center justify-between gap-2 border-t border-zinc-900 pt-2 text-[16px] uppercase tracking-[0.1em] text-zinc-700">
                <span>Source Edition · {dialogTransaction?.source_id ?? '-'}</span>
                <span>{detailSeenRelative}</span>
              </div>
            </div>

            <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
              <section className="news-detail-section">
                <div className="news-kicker border-b border-zinc-900 pb-1">
                  Ledger Entry
                </div>
                <dl className="news-detail-kv mt-3">
                  <dt>Sender</dt>
                  <dd
                    className="news-mono text-[18px] whitespace-nowrap overflow-hidden text-ellipsis"
                    title={detailSender}
                  >
                    {detailSender}
                  </dd>
                  <dt>Tx Type</dt>
                  <dd>{dialogTransaction?.tx_type ?? '-'}</dd>
                  <dt>Seen At</dt>
                  <dd>{detailSeenAt}</dd>
                  <dt>Method</dt>
                  <dd className="news-mono text-[18px]">{dialogFeature?.method_selector ?? '-'}</dd>
                </dl>
              </section>
              <section className="news-detail-section">
                <div className="news-kicker border-b border-zinc-900 pb-1">
                  Wire Details
                </div>
                <dl className="news-detail-kv mt-3">
                  <dt>Nonce</dt>
                  <dd>{dialogTransaction?.nonce ?? dialogDetail?.nonce ?? '-'}</dd>
                  <dt>Peer</dt>
                  <dd>{dialogDetail?.peer ?? '-'}</dd>
                  <dt>Seen Count</dt>
                  <dd>{dialogDetail?.seen_count ?? '-'}</dd>
                  <dt>Payload</dt>
                  <dd>{dialogDetail?.raw_tx_len ?? '-'} bytes</dd>
                </dl>
              </section>
            </div>

            <section className="news-detail-section mt-4">
              <div className="news-kicker border-b border-zinc-900 pb-1">
                Feature Engine
              </div>
              <div className="mt-3 grid grid-cols-2 gap-3 text-[18px]">
                <div>
                  <div className="news-kicker">Protocol</div>
                  <div>{dialogFeature?.protocol ?? '-'}</div>
                </div>
                <div>
                  <div className="news-kicker">Category</div>
                  <div>{dialogFeature?.category ?? '-'}</div>
                </div>
                <div>
                  <div className="news-kicker">MEV Score</div>
                  <div>{dialogFeature?.mev_score ?? '-'}</div>
                </div>
                <div>
                  <div className="news-kicker">Urgency Score</div>
                  <div>{dialogFeature?.urgency_score ?? '-'}</div>
                </div>
                <div className="col-span-2">
                  <div className="news-kicker">Method Selector</div>
                  <div className="news-mono text-[18px]">{dialogFeature?.method_selector ?? '-'}</div>
                </div>
              </div>
            </section>

            <div className="mt-4 min-h-[2.5rem]">
              {dialogError ? (
                <div className="news-mono border border-rose-900 bg-rose-100/70 px-3 py-2 text-[16px] uppercase tracking-[0.08em] text-rose-800">
                  {dialogError}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
