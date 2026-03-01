import { useEffect } from 'react';
import { resolveMarketStatsSnapshot } from '../domain/market-stats.js';
import { normalizeScreenId } from '../domain/screen-mode.js';
import {
  buildDashboardSnapshotPath,
  resolveDashboardSnapshotLimits,
} from '../domain/snapshot-profile.js';
import { resolveDashboardRuntimePolicy } from '../domain/screen-runtime-policy.js';
import {
  fetchJson,
  isAbortError,
  normalizeChainStatusRows,
  opportunityRowKey,
} from '../lib/dashboard-helpers.js';
import {
  clearTransactionDetailCache,
  setTransactionDetailByHash,
} from '../domain/dashboard-stream-store.js';
import { createDashboardPerfMonitor } from '../lib/perf-metrics.js';
import { createSystemHealthMonitor } from '../lib/system-health.js';
import { useDashboardEventStream } from './use-dashboard-event-stream.js';
import { shouldApplyStreamBatch, shouldScheduleGapResync } from '../workers/stream-protocol.js';

const snapshotThrottleMs = 1200;
const streamBatchLimit = 20;
const streamIntervalMs = 1000;
const streamInitialReconnectMs = 1000;
const streamMaxReconnectMs = 30000;
const steadySnapshotRefreshMs = 15000;

function resolvePerfNow() {
  if (typeof globalThis?.performance?.now === 'function') {
    return globalThis.performance.now();
  }
  return Date.now();
}

function resolveUsedHeapSize() {
  const heapBytes = globalThis?.performance?.memory?.usedJSHeapSize;
  if (!Number.isFinite(heapBytes)) {
    return null;
  }
  return heapBytes;
}

function rowsReferenceEqual(left, right) {
  if (left.length !== right.length) {
    return false;
  }
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) {
      return false;
    }
  }
  return true;
}

function stabilizeRows(currentRows, nextRows, keyOf, isEquivalent) {
  if (!Array.isArray(nextRows)) {
    return [];
  }
  if (!Array.isArray(currentRows) || currentRows.length === 0 || nextRows.length === 0) {
    return nextRows;
  }

  const currentByKey = new Map();
  for (const row of currentRows) {
    currentByKey.set(keyOf(row), row);
  }

  const stabilized = nextRows.map((row) => {
    const previous = currentByKey.get(keyOf(row));
    if (previous && isEquivalent(previous, row)) {
      return previous;
    }
    return row;
  });

  return rowsReferenceEqual(currentRows, stabilized) ? currentRows : stabilized;
}

function featureSummaryEquivalent(left, right) {
  return (
    left.protocol === right.protocol
    && left.category === right.category
    && left.count === right.count
  );
}

function featureDetailEquivalent(left, right) {
  return (
    left.hash === right.hash
    && left.protocol === right.protocol
    && left.category === right.category
    && left.chain_id === right.chain_id
    && left.mev_score === right.mev_score
    && left.urgency_score === right.urgency_score
    && left.method_selector === right.method_selector
    && left.feature_engine_version === right.feature_engine_version
  );
}

function txSummaryEquivalent(left, right) {
  return (
    left.hash === right.hash
    && left.sender === right.sender
    && left.nonce === right.nonce
    && left.tx_type === right.tx_type
    && left.seen_unix_ms === right.seen_unix_ms
    && left.source_id === right.source_id
    && left.chain_id === right.chain_id
  );
}

function chainStatusEquivalent(left, right) {
  return (
    left.chain_key === right.chain_key
    && left.chain_id === right.chain_id
    && left.source_id === right.source_id
    && left.state === right.state
    && left.endpoint_index === right.endpoint_index
    && left.endpoint_count === right.endpoint_count
    && left.ws_url === right.ws_url
    && left.http_url === right.http_url
    && left.last_pending_unix_ms === right.last_pending_unix_ms
    && left.silent_for_ms === right.silent_for_ms
    && left.updated_unix_ms === right.updated_unix_ms
    && left.last_error === right.last_error
    && left.rotation_count === right.rotation_count
  );
}

function mergeFeatureDetailRows(existingRows, incomingRows, maxItems) {
  const cap = Number.isFinite(maxItems)
    ? Math.max(1, Math.floor(maxItems))
    : Number.POSITIVE_INFINITY;
  const nextRows = [];
  const seen = new Set();
  const existingByHash = new Map();

  for (const row of existingRows ?? []) {
    if (row?.hash) {
      existingByHash.set(row.hash, row);
    }
  }

  const appendRows = (rows) => {
    for (const row of rows ?? []) {
      const hash = row?.hash;
      if (!hash || seen.has(hash)) {
        continue;
      }
      seen.add(hash);
      const previous = existingByHash.get(hash);
      nextRows.push(previous && featureDetailEquivalent(previous, row) ? previous : row);
      if (nextRows.length >= cap) {
        return true;
      }
    }
    return false;
  };

  if (appendRows(incomingRows)) {
    return nextRows;
  }
  appendRows(existingRows);
  return nextRows;
}

function mergeOpportunityRows(existingRows, incomingRows, maxItems) {
  const cap = Number.isFinite(maxItems)
    ? Math.max(1, Math.floor(maxItems))
    : Number.POSITIVE_INFINITY;
  const nextRows = [];
  const seen = new Set();
  const existingByKey = new Map();

  for (const row of existingRows ?? []) {
    const key = opportunityRowKey(row);
    if (key) {
      existingByKey.set(key, row);
    }
  }

  const appendRows = (rows) => {
    for (const row of rows ?? []) {
      const key = opportunityRowKey(row);
      if (!key || seen.has(key)) {
        continue;
      }
      seen.add(key);
      const previous = existingByKey.get(key);
      nextRows.push(previous && opportunityEquivalent(previous, row) ? previous : row);
      if (nextRows.length >= cap) {
        return true;
      }
    }
    return false;
  };

  const orderedIncoming = [...(incomingRows ?? [])].sort(
    (left, right) => Number(right?.detected_unix_ms ?? 0) - Number(left?.detected_unix_ms ?? 0),
  );
  if (appendRows(orderedIncoming)) {
    return nextRows;
  }
  appendRows(existingRows);
  return nextRows;
}

function reasonsEquivalent(leftReasons, rightReasons) {
  if (leftReasons === rightReasons) {
    return true;
  }
  if (!Array.isArray(leftReasons) || !Array.isArray(rightReasons)) {
    return false;
  }
  if (leftReasons.length !== rightReasons.length) {
    return false;
  }
  for (let index = 0; index < leftReasons.length; index += 1) {
    if (leftReasons[index] !== rightReasons[index]) {
      return false;
    }
  }
  return true;
}

function opportunityEquivalent(left, right) {
  return (
    left.tx_hash === right.tx_hash
    && left.status === right.status
    && left.strategy === right.strategy
    && left.score === right.score
    && left.protocol === right.protocol
    && left.category === right.category
    && left.chain_id === right.chain_id
    && left.source_id === right.source_id
    && left.feature_engine_version === right.feature_engine_version
    && left.scorer_version === right.scorer_version
    && left.strategy_version === right.strategy_version
    && left.detected_unix_ms === right.detected_unix_ms
    && reasonsEquivalent(left.reasons, right.reasons)
  );
}

export function resolveDashboardStreamConnector({
  streamTransport,
  connectEventSource,
  connectWorker,
}) {
  void streamTransport;
  void connectWorker;
  return connectEventSource;
}

export function applyDashboardSseResetResync({
  cancelled = false,
  reset,
  latestSeqIdRef,
  scheduleSnapshot,
  forceTxWindowSnapshot = false,
}) {
  if (cancelled) {
    return;
  }
  const latestSeqId = Number(reset?.latestSeqId);
  if (Number.isFinite(latestSeqId) && latestSeqIdRef) {
    latestSeqIdRef.current = Math.max(
      Number(latestSeqIdRef.current) || 0,
      Math.floor(latestSeqId),
    );
  }
  scheduleSnapshot?.('sse-reset-resync', true, {
    forceTxWindow: Boolean(forceTxWindowSnapshot),
  });
}

export function useDashboardLifecycleEffects({
  apiBase,
  snapshotTxLimit,
  maxTransactionHistory,
  maxFeatureHistory,
  maxOpportunityHistory,
  streamBatchMs,
  streamTransport,
  samplingLagThresholdMs,
  samplingStride,
  samplingFlushIdleMs,
  heapEmergencyPurgeMb,
  devPerformanceEntryCleanupEnabled,
  devPerformanceEntryCleanupIntervalMs,
  workerEnabled,
  transactionRetentionMs,
  transactionRetentionMinutes,
  detailCacheLimit,
  activeScreen,
  dialogHash,
  closeDialog,
  followLatest,
  recentTxRows,
  transactionRows,
  query,
  liveMainnetFilter,
  recentTxRowsRef,
  transactionRowsRef,
  transactionStoreRef,
  followLatestRef,
  latestSeqIdRef,
  lastGapSnapshotAtRef,
  reconnectAttemptsRef,
  reconnectTimerRef,
  snapshotTimerRef,
  snapshotInFlightRef,
  nextSnapshotAtRef,
  setTransactionPage,
  setActiveScreen,
  setOpportunityRows,
  setFeatureSummaryRows,
  setFeatureDetailRows,
  setChainStatusRows,
  setMarketStats,
  enqueueTickerCommit,
  flushTickerCommit,
  cancelTickerCommit,
  resolveTransactionDetail,
  setSelectedOpportunityKey,
  setStatusMessage,
  setHasError,
  setDialogLoading,
  setDialogError,
}) {
  const { connectEventSource, disconnectEventSource } = useDashboardEventStream();

  useEffect(() => {
    recentTxRowsRef.current = recentTxRows;
  }, [recentTxRows, recentTxRowsRef]);

  useEffect(() => {
    transactionRowsRef.current = transactionRows;
  }, [transactionRows, transactionRowsRef]);

  useEffect(() => {
    followLatestRef.current = followLatest;
  }, [followLatest, followLatestRef]);

  useEffect(() => {
    if (followLatest) {
      setTransactionPage(1);
    }
  }, [followLatest, setTransactionPage, transactionRows]);

  useEffect(() => {
    setTransactionPage(1);
  }, [liveMainnetFilter, query, setTransactionPage]);

  useEffect(() => {
    const onHashChange = () => {
      setActiveScreen(normalizeScreenId(window.location.hash.replace(/^#/, '')));
    };
    window.addEventListener('hashchange', onHashChange);
    return () => {
      window.removeEventListener('hashchange', onHashChange);
    };
  }, [setActiveScreen]);

  useEffect(() => {
    const expectedHash = `#${activeScreen}`;
    if (window.location.hash !== expectedHash) {
      window.history.replaceState(null, '', expectedHash);
    }
  }, [activeScreen]);

  useEffect(() => {
    let cancelled = false;
    const perfMonitor = createDashboardPerfMonitor(globalThis?.window);
    const resolvedStreamBatchMs = Number.isFinite(streamBatchMs)
      ? Math.max(100, Math.floor(streamBatchMs))
      : 500;
    const runtimePolicy = resolveDashboardRuntimePolicy(activeScreen);
    let heapSampleTimer = null;
    let fallbackSnapshotPollTimer = null;
    let steadySnapshotTimer = null;
    let snapshotRequestAbortController = null;
    let sampledCommit = null;
    let streamBatchIndex = 0;
    let frameProbeRafId = null;
    let lastAnimationFrameTs = null;
    let performanceEntryCleanupTimer = null;
    let pendingDeltaFlushCancel = null;
    let pendingDeltaTransactions = [];
    let pendingDeltaFeatureRows = [];
    let pendingDeltaOpportunityRows = [];
    let pendingDeltaMarketStats = null;
    let pendingDeltaLatestSeqId = null;

    const cleanupStreamConnections = () => {
      disconnectEventSource();
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

    const clearHeapSampleTimer = () => {
      if (heapSampleTimer) {
        window.clearInterval(heapSampleTimer);
        heapSampleTimer = null;
      }
    };

    const clearFallbackSnapshotPollTimer = () => {
      if (fallbackSnapshotPollTimer) {
        window.clearInterval(fallbackSnapshotPollTimer);
        fallbackSnapshotPollTimer = null;
      }
    };

    const clearSteadySnapshotTimer = () => {
      if (steadySnapshotTimer) {
        window.clearInterval(steadySnapshotTimer);
        steadySnapshotTimer = null;
      }
    };

    const abortSnapshotRequest = () => {
      if (snapshotRequestAbortController) {
        snapshotRequestAbortController.abort();
        snapshotRequestAbortController = null;
      }
    };

    const clearFrameProbe = () => {
      if (
        frameProbeRafId != null
        && typeof window.cancelAnimationFrame === 'function'
      ) {
        window.cancelAnimationFrame(frameProbeRafId);
      }
      frameProbeRafId = null;
      lastAnimationFrameTs = null;
    };

    const clearPerformanceEntryCleanupTimer = () => {
      if (performanceEntryCleanupTimer) {
        window.clearInterval(performanceEntryCleanupTimer);
        performanceEntryCleanupTimer = null;
      }
    };

    const clearPerformanceEntries = () => {
      if (!import.meta.env.DEV || !devPerformanceEntryCleanupEnabled) {
        return;
      }
      const perf = globalThis?.performance;
      if (!perf || typeof perf !== 'object') {
        return;
      }
      if (typeof perf.clearMeasures === 'function') {
        perf.clearMeasures();
      }
      if (typeof perf.clearMarks === 'function') {
        perf.clearMarks();
      }
      if (typeof perf.clearResourceTimings === 'function') {
        perf.clearResourceTimings();
      }
    };

    const appendPendingRows = (targetRows, incomingRows) => {
      if (!Array.isArray(incomingRows) || incomingRows.length === 0) {
        return;
      }
      for (const row of incomingRows) {
        targetRows.push(row);
      }
    };

    const cancelPendingDeltaFlush = () => {
      if (typeof pendingDeltaFlushCancel === 'function') {
        pendingDeltaFlushCancel();
      }
      pendingDeltaFlushCancel = null;
    };

    const resetPendingStreamDelta = () => {
      cancelPendingDeltaFlush();
      pendingDeltaTransactions = [];
      pendingDeltaFeatureRows = [];
      pendingDeltaOpportunityRows = [];
      pendingDeltaMarketStats = null;
      pendingDeltaLatestSeqId = null;
    };

    const flushSampledCommit = () => {
      if (!sampledCommit) {
        return false;
      }
      const queuedAtMs = resolvePerfNow();
      enqueueTickerCommit(sampledCommit, {
        requestFrame: typeof window.requestAnimationFrame === 'function'
          ? (onFrame) => window.requestAnimationFrame((rafTimestampMs) => {
            const frameTimestampMs = Number.isFinite(rafTimestampMs)
              ? rafTimestampMs
              : resolvePerfNow();
            perfMonitor.recordTransactionCommit(frameTimestampMs - queuedAtMs);
            onFrame(frameTimestampMs);
          })
          : null,
        cancelFrame: typeof window.cancelAnimationFrame === 'function'
          ? window.cancelAnimationFrame.bind(window)
          : null,
      });
      sampledCommit = null;
      return true;
    };

    const emergencyPurge = (usedHeapBytes) => {
      if (cancelled) {
        return;
      }

      resetPendingStreamDelta();
      sampledCommit = null;
      streamBatchIndex = 0;
      cancelTickerCommit();
      transactionStoreRef.current.reset();
      clearTransactionDetailCache();
      recentTxRowsRef.current = [];
      transactionRowsRef.current = [];
      enqueueTickerCommit({
        recentTxRows: [],
        transactionRows: [],
      });
      flushTickerCommit();
      setOpportunityRows([]);
      setFeatureSummaryRows([]);
      setFeatureDetailRows([]);
      setChainStatusRows([]);
      setSelectedOpportunityKey(null);
      latestSeqIdRef.current = 0;

      const heapMb = Math.round(usedHeapBytes / (1024 * 1024));
      setStatusMessage(
        `Memory pressure detected (${heapMb}MB). Purging buffers and forcing snapshot sync...`,
      );
      setHasError(true);
    };

    const systemHealth = createSystemHealthMonitor({
      samplingLagThresholdMs,
      samplingStride,
      samplingFlushIdleMs,
      heapEmergencyPurgeMb,
      onEmergencyPurge: emergencyPurge,
    });

    const startFrameProbe = () => {
      if (typeof window.requestAnimationFrame !== 'function') {
        return;
      }
      const onAnimationFrame = (frameTs) => {
        if (cancelled) {
          return;
        }
        if (lastAnimationFrameTs != null) {
          const frameDeltaMs = frameTs - lastAnimationFrameTs;
          if (Number.isFinite(frameDeltaMs) && frameDeltaMs >= 0) {
            perfMonitor.recordFrameDuration(frameDeltaMs, frameTs);
            systemHealth.recordFrameDelta(frameDeltaMs);
          }
        }
        lastAnimationFrameTs = frameTs;
        frameProbeRafId = window.requestAnimationFrame(onAnimationFrame);
      };
      frameProbeRafId = window.requestAnimationFrame(onAnimationFrame);
    };

    const captureHeapSample = () => {
      const heapBytes = resolveUsedHeapSize();
      if (heapBytes != null) {
        perfMonitor.recordHeapSample(heapBytes);
        if (systemHealth.recordHeapBytes(heapBytes)) {
          scheduleSnapshot('heap-emergency-purge', true);
        }
      }
    };

    const clearTransactionBuffers = () => {
      resetPendingStreamDelta();
      sampledCommit = null;
      streamBatchIndex = 0;
      cancelTickerCommit();
      transactionStoreRef.current.reset();
      recentTxRowsRef.current = [];
      transactionRowsRef.current = [];
      enqueueTickerCommit({
        recentTxRows: [],
        transactionRows: [],
      });
      flushTickerCommit();
      setFeatureSummaryRows((current) => (current.length ? [] : current));
      setFeatureDetailRows((current) => (current.length ? [] : current));
    };

    const flushTransactionCommit = () => {
      if (cancelled) {
        return;
      }
      if (flushSampledCommit()) {
        return;
      }
      const commitStartedAtMs = resolvePerfNow();
      flushTickerCommit();
      const commitEndedAtMs = resolvePerfNow();
      perfMonitor.recordTransactionCommit(commitEndedAtMs - commitStartedAtMs);
    };

    const queueTransactionCommit = (commit) => {
      sampledCommit = commit;
      streamBatchIndex += 1;
      if (systemHealth.shouldRenderBatch(streamBatchIndex)) {
        systemHealth.clearTrailingFlush();
        flushSampledCommit();
        return;
      }
      systemHealth.scheduleTrailingFlush(() => {
        if (cancelled) {
          return;
        }
        flushTransactionCommit();
      });
    };

    const commitMarketStats = (rawStats) => {
      if (!rawStats || typeof rawStats !== 'object') {
        return;
      }
      setMarketStats((current) => {
        const next = resolveMarketStatsSnapshot(rawStats, current);
        if (
          next.totalSignalVolume === current.totalSignalVolume
          && next.totalTxCount === current.totalTxCount
          && next.lowRiskCount === current.lowRiskCount
          && next.mediumRiskCount === current.mediumRiskCount
          && next.highRiskCount === current.highRiskCount
          && next.successRate === current.successRate
        ) {
          return current;
        }
        return next;
      });
    };

    const applyDeltaTransactions = (transactions, sourceLabel, latestSeqId) => {
      const incoming = Array.isArray(transactions) ? transactions : [];
      if (!incoming.length) {
        if (Number.isFinite(latestSeqId)) {
          latestSeqIdRef.current = Math.max(latestSeqIdRef.current, latestSeqId);
        }
        setStatusMessage(
          `Streaming(${sourceLabel}) · tx=${transactionRowsRef.current.length}/${maxTransactionHistory} · window=${transactionRetentionMinutes}m`,
        );
        return;
      }

      const orderedIncoming = [...incoming].sort(
        (left, right) => Number(right?.seen_unix_ms ?? 0) - Number(left?.seen_unix_ms ?? 0),
      );
      const txSnapshot = transactionStoreRef.current.snapshot(orderedIncoming, {
        maxItems: maxTransactionHistory,
        nowUnixMs: Date.now(),
        maxAgeMs: transactionRetentionMs,
      });
      const nextTransactions = txSnapshot.rows;
      const stabilizedTransactions = rowsReferenceEqual(
        transactionRowsRef.current,
        nextTransactions,
      )
        ? transactionRowsRef.current
        : nextTransactions;
      transactionRowsRef.current = stabilizedTransactions;

      const nextRecent = stabilizedTransactions.slice(
        0,
        Math.min(snapshotTxLimit, stabilizedTransactions.length),
      );
      const stabilizedRecent = rowsReferenceEqual(recentTxRowsRef.current, nextRecent)
        ? recentTxRowsRef.current
        : nextRecent;
      recentTxRowsRef.current = stabilizedRecent;

      queueTransactionCommit({
        recentTxRows: stabilizedRecent,
        transactionRows: stabilizedTransactions,
      });

      if (Number.isFinite(latestSeqId)) {
        latestSeqIdRef.current = Math.max(latestSeqIdRef.current, latestSeqId);
      }
      setStatusMessage(
        `Streaming(${sourceLabel}) · tx=${stabilizedTransactions.length}/${maxTransactionHistory} · window=${transactionRetentionMinutes}m`,
      );
    };

    const applyDeltaFeatureDetails = (featureRows) => {
      const incoming = Array.isArray(featureRows) ? featureRows : [];
      if (!incoming.length) {
        return;
      }
      setFeatureDetailRows((current) => {
        const merged = mergeFeatureDetailRows(current, incoming, maxFeatureHistory);
        return rowsReferenceEqual(current, merged) ? current : merged;
      });
    };

    const applyDeltaOpportunities = (opportunityRows) => {
      const incoming = Array.isArray(opportunityRows) ? opportunityRows : [];
      if (!incoming.length) {
        return;
      }
      setOpportunityRows((current) => {
        const merged = mergeOpportunityRows(current, incoming, maxOpportunityHistory);
        return rowsReferenceEqual(current, merged) ? current : merged;
      });
    };

    const flushPendingStreamDelta = () => {
      const nextMarketStats = pendingDeltaMarketStats;
      const nextOpportunityRows = pendingDeltaOpportunityRows;
      const nextFeatureRows = pendingDeltaFeatureRows;
      const nextTransactions = pendingDeltaTransactions;
      const nextLatestSeqId = pendingDeltaLatestSeqId;
      pendingDeltaMarketStats = null;
      pendingDeltaOpportunityRows = [];
      pendingDeltaFeatureRows = [];
      pendingDeltaTransactions = [];
      pendingDeltaLatestSeqId = null;

      if (runtimePolicy.shouldComputeRadarDerived) {
        commitMarketStats(nextMarketStats);
      }
      applyDeltaOpportunities(nextOpportunityRows);
      if (runtimePolicy.shouldProcessTxStream) {
        applyDeltaFeatureDetails(nextFeatureRows);
        if (nextTransactions.length || Number.isFinite(nextLatestSeqId)) {
          applyDeltaTransactions(nextTransactions, 'ws-delta', nextLatestSeqId);
        }
      } else if (Number.isFinite(nextLatestSeqId)) {
        latestSeqIdRef.current = Math.max(latestSeqIdRef.current, nextLatestSeqId);
      }
    };

    const schedulePendingStreamDeltaFlush = () => {
      if (pendingDeltaFlushCancel || cancelled) {
        return;
      }

      if (
        typeof window.requestAnimationFrame === 'function'
        && typeof window.cancelAnimationFrame === 'function'
      ) {
        const rafId = window.requestAnimationFrame(() => {
          pendingDeltaFlushCancel = null;
          flushPendingStreamDelta();
        });
        pendingDeltaFlushCancel = () => {
          window.cancelAnimationFrame(rafId);
        };
        return;
      }

      const timeoutId = window.setTimeout(() => {
        pendingDeltaFlushCancel = null;
        flushPendingStreamDelta();
      }, 0);
      pendingDeltaFlushCancel = () => {
        window.clearTimeout(timeoutId);
      };
    };

    const applySnapshot = (snapshot, sourceLabel) => {
      const applyStartedAt = resolvePerfNow();
      const opportunities = Array.isArray(snapshot?.opportunities)
        ? snapshot.opportunities.slice(0, maxOpportunityHistory)
        : [];
      const featureSummary = runtimePolicy.shouldComputeRadarDerived && Array.isArray(snapshot?.feature_summary)
        ? snapshot.feature_summary
        : [];
      const featureDetails = runtimePolicy.shouldComputeRadarDerived && Array.isArray(snapshot?.feature_details)
        ? snapshot.feature_details.slice(0, maxFeatureHistory)
        : [];
      const txRecent = runtimePolicy.shouldProcessTxStream && Array.isArray(snapshot?.transactions)
        ? snapshot.transactions
        : [];
      const chainStatus = normalizeChainStatusRows(snapshot?.chain_ingest_status);

      let transactionCount = transactionRowsRef.current.length;
      if (runtimePolicy.shouldProcessTxStream) {
        const txSnapshot = transactionStoreRef.current.snapshot(txRecent, {
          maxItems: maxTransactionHistory,
          nowUnixMs: Date.now(),
          maxAgeMs: transactionRetentionMs,
        });
        const nextTransactions = txSnapshot.rows;
        const stabilizedTransactions = rowsReferenceEqual(
          transactionRowsRef.current,
          nextTransactions,
        )
          ? transactionRowsRef.current
          : nextTransactions;
        transactionRowsRef.current = stabilizedTransactions;

        const stabilizedRecentTxRows = stabilizeRows(
          recentTxRowsRef.current,
          txRecent,
          (row) => row.hash,
          txSummaryEquivalent,
        );
        recentTxRowsRef.current = stabilizedRecentTxRows;

        queueTransactionCommit({
          recentTxRows: stabilizedRecentTxRows,
          transactionRows: stabilizedTransactions,
        });
        transactionCount = stabilizedTransactions.length;
      }
      setOpportunityRows((current) =>
        stabilizeRows(current, opportunities, opportunityRowKey, opportunityEquivalent),
      );
      if (runtimePolicy.shouldComputeRadarDerived) {
        setFeatureSummaryRows((current) =>
          stabilizeRows(
            current,
            featureSummary,
            (row) => `${row.protocol}::${row.category}`,
            featureSummaryEquivalent,
          ),
        );
        setFeatureDetailRows((current) =>
          stabilizeRows(current, featureDetails, (row) => row.hash, featureDetailEquivalent),
        );
      } else {
        setFeatureSummaryRows((current) => (current.length ? [] : current));
        setFeatureDetailRows((current) => (current.length ? [] : current));
      }
      setChainStatusRows((current) =>
        stabilizeRows(current, chainStatus, (row) => row.chain_key, chainStatusEquivalent),
      );
      if (runtimePolicy.shouldComputeRadarDerived) {
        commitMarketStats(snapshot?.market_stats);
      }

      setSelectedOpportunityKey((current) => {
        if (current && opportunities.some((row) => opportunityRowKey(row) === current)) {
          return current;
        }
        return opportunities[0] ? opportunityRowKey(opportunities[0]) : null;
      });

      const latestSeqId = Number.isFinite(snapshot?.latest_seq_id)
        ? snapshot.latest_seq_id
        : 0;
      latestSeqIdRef.current = Math.max(latestSeqIdRef.current, latestSeqId);

      const lastUpdated = new Date().toLocaleTimeString();
      setStatusMessage(
        `Connected(${sourceLabel}) · opps=${opportunities.length} · features=${featureDetails.length} · tx=${transactionCount}/${maxTransactionHistory} · window=${transactionRetentionMinutes}m · ${lastUpdated}`,
      );
      perfMonitor.recordSnapshotApply(resolvePerfNow() - applyStartedAt);
      captureHeapSample();
    };

    const syncSnapshot = async (sourceLabel, options = {}) => {
      if (cancelled) {
        return;
      }
      if (snapshotInFlightRef.current) {
        perfMonitor.markSnapshotDeferred();
        return;
      }
      const forceTxWindow = Boolean(options?.forceTxWindow);
      const effectiveSnapshotPath = buildDashboardSnapshotPath(
        resolveDashboardSnapshotLimits({
          activeScreen,
          snapshotTxLimit: forceTxWindow ? maxTransactionHistory : snapshotTxLimit,
        }),
      );
      const requestAbortController = new AbortController();
      snapshotRequestAbortController = requestAbortController;
      snapshotInFlightRef.current = true;
      perfMonitor.beginSnapshotRequest();
      let requestStatus = 'completed';
      try {
        const snapshot = await fetchJson(apiBase, effectiveSnapshotPath, {
          signal: requestAbortController.signal,
        });
        if (cancelled || snapshotRequestAbortController !== requestAbortController) {
          requestStatus = 'aborted';
          return;
        }
        setHasError(false);
        applySnapshot(snapshot, sourceLabel);
      } catch (error) {
        if (
          cancelled
          || snapshotRequestAbortController !== requestAbortController
          || isAbortError(error)
        ) {
          requestStatus = 'aborted';
          return;
        }
        requestStatus = 'failed';
        setHasError(true);
        setStatusMessage(`Snapshot sync failed: ${error.message}`);
      } finally {
        if (snapshotRequestAbortController === requestAbortController) {
          snapshotRequestAbortController = null;
        }
        perfMonitor.finishSnapshotRequest(requestStatus);
        snapshotInFlightRef.current = false;
      }
    };

    const scheduleSnapshot = (sourceLabel, immediate = false, options = {}) => {
      if (cancelled) {
        return;
      }
      if (snapshotTimerRef.current) {
        perfMonitor.markSnapshotDeferred();
        return;
      }
      const now = Date.now();
      const delay = immediate
        ? 0
        : Math.max(0, nextSnapshotAtRef.current - now);
      snapshotTimerRef.current = window.setTimeout(async () => {
        snapshotTimerRef.current = null;
        nextSnapshotAtRef.current = Date.now() + snapshotThrottleMs;
        await syncSnapshot(sourceLabel, options);
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

    const handleStreamOpen = (sourceLabel) => {
      if (cancelled) {
        return;
      }
      reconnectAttemptsRef.current = 0;
      setHasError(false);
      scheduleSnapshot(sourceLabel, true, {
        forceTxWindow: runtimePolicy.shouldForceTxWindowSnapshot,
      });
    };

    const handleStreamBatch = (batch) => {
      if (cancelled) {
        return;
      }

      const previousSeqId = Number.isFinite(pendingDeltaLatestSeqId)
        ? Math.max(latestSeqIdRef.current, pendingDeltaLatestSeqId)
        : latestSeqIdRef.current;
      const seqId = batch.latestSeqId;
      if (!shouldApplyStreamBatch({
        previousSeqId,
        latestSeqId: seqId,
        transactionCount: Array.isArray(batch.transactions) ? batch.transactions.length : 0,
        featureCount: Array.isArray(batch.featureRows) ? batch.featureRows.length : 0,
        opportunityCount: Array.isArray(batch.opportunityRows)
          ? batch.opportunityRows.length
          : 0,
      })) {
        pendingDeltaMarketStats = batch.marketStats ?? pendingDeltaMarketStats;
        schedulePendingStreamDeltaFlush();
        return;
      }

      pendingDeltaMarketStats = batch.marketStats ?? pendingDeltaMarketStats;
      appendPendingRows(pendingDeltaOpportunityRows, batch.opportunityRows);
      if (runtimePolicy.shouldProcessTxStream) {
        appendPendingRows(pendingDeltaFeatureRows, batch.featureRows);
        appendPendingRows(pendingDeltaTransactions, batch.transactions);
      }
      if (Number.isFinite(seqId)) {
        pendingDeltaLatestSeqId = Number.isFinite(pendingDeltaLatestSeqId)
          ? Math.max(pendingDeltaLatestSeqId, seqId)
          : seqId;
      }
      schedulePendingStreamDeltaFlush();
      if (shouldScheduleGapResync({
        previousSeqId,
        latestSeqId: seqId,
        hasGap: batch.hasGap,
        nowUnixMs: Date.now(),
        lastResyncUnixMs: lastGapSnapshotAtRef.current,
        cooldownMs: steadySnapshotRefreshMs,
      })) {
        lastGapSnapshotAtRef.current = Date.now();
        scheduleSnapshot('stream-gap-resync', true, {
          forceTxWindow: runtimePolicy.shouldForceTxWindowSnapshot,
        });
      }
    };

    const connectStream = () => {
      if (cancelled) {
        return;
      }
      resetPendingStreamDelta();
      cleanupStreamConnections();
      clearFallbackSnapshotPollTimer();
      clearSteadySnapshotTimer();
      if (!workerEnabled) {
        reconnectAttemptsRef.current = 0;
        const fallbackSnapshotRefreshMs = runtimePolicy.shouldProcessTxStream
          ? snapshotThrottleMs
          : steadySnapshotRefreshMs;
        setStatusMessage(
          `Worker pipeline disabled · polling snapshots every ${fallbackSnapshotRefreshMs}ms`,
        );
        scheduleSnapshot('snapshot-poll-bootstrap', true, {
          forceTxWindow: runtimePolicy.shouldForceTxWindowSnapshot,
        });
        fallbackSnapshotPollTimer = window.setInterval(() => {
          scheduleSnapshot('snapshot-poll', true, {
            forceTxWindow: runtimePolicy.shouldForceTxWindowSnapshot,
          });
        }, fallbackSnapshotRefreshMs);
        return;
      }

      const connectTransport = resolveDashboardStreamConnector({
        streamTransport,
        connectEventSource: () => {
          connectEventSource({
            apiBase,
            afterSeqId: latestSeqIdRef.current,
            onOpen: () => {
              handleStreamOpen('sse-open');
            },
            onBatch: handleStreamBatch,
            onReset: (reset) => {
              applyDashboardSseResetResync({
                cancelled,
                reset,
                latestSeqIdRef,
                scheduleSnapshot,
                forceTxWindowSnapshot: runtimePolicy.shouldForceTxWindowSnapshot,
              });
            },
            onError: (error) => {
              if (cancelled) {
                return;
              }
              setHasError(true);
              setStatusMessage(`SSE stream error: ${error.message}`);
            },
          });
        },
        connectWorker: () => {},
      });
      connectTransport?.();
    };

    setStatusMessage(`Connecting events(v1/sse) to ${apiBase}`);
    setHasError(false);
    if (!runtimePolicy.shouldProcessTxStream) {
      clearTransactionBuffers();
    }
    startFrameProbe();
    captureHeapSample();
    heapSampleTimer = window.setInterval(() => {
      captureHeapSample();
    }, 5000);
    if (import.meta.env.DEV && devPerformanceEntryCleanupEnabled) {
      clearPerformanceEntries();
      performanceEntryCleanupTimer = window.setInterval(() => {
        clearPerformanceEntries();
      }, devPerformanceEntryCleanupIntervalMs);
    }
    scheduleSnapshot('bootstrap', true, {
      forceTxWindow: runtimePolicy.shouldForceTxWindowSnapshot,
    });
    connectStream();
    if (workerEnabled && runtimePolicy.shouldProcessTxStream) {
      steadySnapshotTimer = window.setInterval(() => {
        scheduleSnapshot('steady-refresh', true);
      }, steadySnapshotRefreshMs);
    }

    return () => {
      resetPendingStreamDelta();
      flushTransactionCommit();
      cancelled = true;
      abortSnapshotRequest();
      cancelTickerCommit();
      systemHealth.clearTrailingFlush();
      clearReconnectTimer();
      clearSnapshotTimer();
      clearHeapSampleTimer();
      clearFallbackSnapshotPollTimer();
      clearSteadySnapshotTimer();
      clearFrameProbe();
      clearPerformanceEntryCleanupTimer();
      cleanupStreamConnections();
    };
  }, [
    apiBase,
    activeScreen,
    connectEventSource,
    enqueueTickerCommit,
    flushTickerCommit,
    cancelTickerCommit,
    disconnectEventSource,
    followLatestRef,
    latestSeqIdRef,
    lastGapSnapshotAtRef,
    maxFeatureHistory,
    maxOpportunityHistory,
    maxTransactionHistory,
    streamBatchMs,
    streamTransport,
    samplingLagThresholdMs,
    samplingStride,
    samplingFlushIdleMs,
    heapEmergencyPurgeMb,
    devPerformanceEntryCleanupEnabled,
    devPerformanceEntryCleanupIntervalMs,
    workerEnabled,
    nextSnapshotAtRef,
    reconnectAttemptsRef,
    reconnectTimerRef,
    recentTxRowsRef,
    setChainStatusRows,
    setFeatureDetailRows,
    setFeatureSummaryRows,
    setHasError,
    setMarketStats,
    setOpportunityRows,
    resolveTransactionDetail,
    setSelectedOpportunityKey,
    setStatusMessage,
    snapshotInFlightRef,
    snapshotTimerRef,
    snapshotTxLimit,
    transactionStoreRef,
    transactionRetentionMinutes,
    transactionRetentionMs,
    transactionRowsRef,
  ]);

  useEffect(() => {
    if (!dialogHash) {
      return undefined;
    }
    if (resolveTransactionDetail(dialogHash)) {
      return undefined;
    }

    let cancelled = false;
    const detailRequestAbortController = new AbortController();
    const perfApi = globalThis?.window?.__MEMPULSE_PERF__;
    const beginDetailRequest = () => {
      if (typeof perfApi?.beginDetailRequest === 'function') {
        perfApi.beginDetailRequest();
      }
    };
    const finishDetailRequest = (status) => {
      if (typeof perfApi?.finishDetailRequest === 'function') {
        perfApi.finishDetailRequest(status);
      }
    };
    setDialogLoading(true);
    setDialogError('');
    beginDetailRequest();
    let detailRequestStatus = 'completed';

    fetchJson(apiBase, `/transactions/${encodeURIComponent(dialogHash)}`, {
      signal: detailRequestAbortController.signal,
    })
      .then((detail) => {
        if (cancelled) {
          detailRequestStatus = 'aborted';
          return;
        }
        setTransactionDetailByHash(dialogHash, detail, detailCacheLimit);
        setDialogLoading(false);
      })
      .catch((error) => {
        if (cancelled || isAbortError(error)) {
          detailRequestStatus = 'aborted';
          return;
        }
        detailRequestStatus = 'failed';
        setDialogLoading(false);
        setDialogError(`Failed to load tx details: ${error.message}`);
      })
      .finally(() => {
        finishDetailRequest(detailRequestStatus);
      });

    return () => {
      cancelled = true;
      detailRequestAbortController.abort();
    };
  }, [
    apiBase,
    detailCacheLimit,
    dialogHash,
    resolveTransactionDetail,
    setDialogError,
    setDialogLoading,
  ]);

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
}
