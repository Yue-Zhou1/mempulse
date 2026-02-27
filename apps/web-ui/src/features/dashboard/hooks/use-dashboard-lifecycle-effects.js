import { useEffect } from 'react';
import { resolveMarketStatsSnapshot } from '../domain/market-stats.js';
import { normalizeScreenId } from '../domain/screen-mode.js';
import {
  buildDashboardSnapshotPath,
  resolveDashboardSnapshotLimits,
} from '../domain/snapshot-profile.js';
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
import { useDashboardStreamWorker } from './use-dashboard-stream-worker.js';
import { shouldScheduleGapResync } from '../workers/stream-protocol.js';

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

export function useDashboardLifecycleEffects({
  apiBase,
  snapshotTxLimit,
  maxTransactionHistory,
  maxFeatureHistory,
  maxOpportunityHistory,
  streamBatchMs,
  samplingLagThresholdMs,
  samplingStride,
  samplingFlushIdleMs,
  heapEmergencyPurgeMb,
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
  const { connectWorker, disconnectWorker } = useDashboardStreamWorker();

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
    const snapshotPath = buildDashboardSnapshotPath(
      resolveDashboardSnapshotLimits({ activeScreen, snapshotTxLimit }),
    );
    const perfMonitor = createDashboardPerfMonitor(globalThis?.window);
    const resolvedStreamBatchMs = Number.isFinite(streamBatchMs)
      ? Math.max(100, Math.floor(streamBatchMs))
      : 500;
    let heapSampleTimer = null;
    let fallbackSnapshotPollTimer = null;
    let steadySnapshotTimer = null;
    let snapshotRequestAbortController = null;
    let sampledCommit = null;
    let streamBatchIndex = 0;
    let frameProbeRafId = null;
    let lastAnimationFrameTs = null;

    const cleanupStreamWorker = () => {
      disconnectWorker();
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

    const applySnapshot = (snapshot, sourceLabel) => {
      const applyStartedAt = resolvePerfNow();
      const opportunities = Array.isArray(snapshot?.opportunities)
        ? snapshot.opportunities.slice(0, maxOpportunityHistory)
        : [];
      const featureSummary = Array.isArray(snapshot?.feature_summary)
        ? snapshot.feature_summary
        : [];
      const featureDetails = Array.isArray(snapshot?.feature_details)
        ? snapshot.feature_details.slice(0, maxFeatureHistory)
        : [];
      const txRecent = Array.isArray(snapshot?.transactions) ? snapshot.transactions : [];
      const chainStatus = normalizeChainStatusRows(snapshot?.chain_ingest_status);

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
      setOpportunityRows((current) =>
        stabilizeRows(current, opportunities, opportunityRowKey, opportunityEquivalent),
      );
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
      setChainStatusRows((current) =>
        stabilizeRows(current, chainStatus, (row) => row.chain_key, chainStatusEquivalent),
      );
      setMarketStats((current) => {
        const next = resolveMarketStatsSnapshot(snapshot?.market_stats, current);
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
        `Connected(${sourceLabel}) · opps=${opportunities.length} · features=${featureDetails.length} · tx=${nextTransactions.length}/${maxTransactionHistory} · window=${transactionRetentionMinutes}m · ${lastUpdated}`,
      );
      perfMonitor.recordSnapshotApply(resolvePerfNow() - applyStartedAt);
      captureHeapSample();
    };

    const syncSnapshot = async (sourceLabel) => {
      if (cancelled || snapshotInFlightRef.current) {
        return;
      }
      const requestAbortController = new AbortController();
      snapshotRequestAbortController = requestAbortController;
      snapshotInFlightRef.current = true;
      try {
        const snapshot = await fetchJson(apiBase, snapshotPath, {
          signal: requestAbortController.signal,
        });
        if (cancelled || snapshotRequestAbortController !== requestAbortController) {
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
          return;
        }
        setHasError(true);
        setStatusMessage(`Snapshot sync failed: ${error.message}`);
      } finally {
        if (snapshotRequestAbortController === requestAbortController) {
          snapshotRequestAbortController = null;
        }
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
      cleanupStreamWorker();
      clearFallbackSnapshotPollTimer();
      if (!workerEnabled) {
        reconnectAttemptsRef.current = 0;
        setStatusMessage(
          `Worker pipeline disabled · polling snapshots every ${snapshotThrottleMs}ms`,
        );
        scheduleSnapshot('snapshot-poll-bootstrap', true);
        fallbackSnapshotPollTimer = window.setInterval(() => {
          scheduleSnapshot('snapshot-poll', true);
        }, snapshotThrottleMs);
        return;
      }
      connectWorker({
        apiBase,
        afterSeqId: latestSeqIdRef.current,
        batchWindowMs: resolvedStreamBatchMs,
        streamBatchLimit,
        streamIntervalMs,
        initialCredit: 1,
        onOpen: () => {
          if (cancelled) {
            return;
          }
          reconnectAttemptsRef.current = 0;
          setHasError(false);
          scheduleSnapshot('ws-open', true);
        },
        onBatch: (batch) => {
          if (cancelled) {
            return;
          }

          const previousSeqId = latestSeqIdRef.current;
          const seqId = batch.latestSeqId;
          if (!Number.isFinite(seqId) || seqId <= previousSeqId) {
            return;
          }

          applyDeltaTransactions(batch.transactions, 'ws-delta', seqId);
          if (shouldScheduleGapResync({
            previousSeqId,
            latestSeqId: seqId,
            hasGap: batch.hasGap,
            nowUnixMs: Date.now(),
            lastResyncUnixMs: lastGapSnapshotAtRef.current,
            cooldownMs: steadySnapshotRefreshMs,
          })) {
            lastGapSnapshotAtRef.current = Date.now();
            scheduleSnapshot('ws-gap-resync', true);
          }
        },
        onClose: () => {
          if (cancelled) {
            return;
          }
          scheduleReconnect();
        },
        onError: (error) => {
          if (cancelled) {
            return;
          }
          setHasError(true);
          setStatusMessage(`Stream worker error: ${error.message}`);
          scheduleReconnect();
        },
      });
    };

    setStatusMessage(
      `Connecting stream to ${apiBase} · cadence=${resolvedStreamBatchMs}ms · batch<=${streamBatchLimit}`,
    );
    setHasError(false);
    startFrameProbe();
    captureHeapSample();
    heapSampleTimer = window.setInterval(() => {
      captureHeapSample();
    }, 5000);
    steadySnapshotTimer = window.setInterval(() => {
      scheduleSnapshot('steady-refresh', true);
    }, steadySnapshotRefreshMs);
    scheduleSnapshot('bootstrap', true);
    connectStream();

    return () => {
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
      cleanupStreamWorker();
    };
  }, [
    apiBase,
    activeScreen,
    connectWorker,
    enqueueTickerCommit,
    flushTickerCommit,
    cancelTickerCommit,
    disconnectWorker,
    followLatestRef,
    latestSeqIdRef,
    lastGapSnapshotAtRef,
    maxFeatureHistory,
    maxOpportunityHistory,
    maxTransactionHistory,
    streamBatchMs,
    samplingLagThresholdMs,
    samplingStride,
    samplingFlushIdleMs,
    heapEmergencyPurgeMb,
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
    setDialogLoading(true);
    setDialogError('');

    fetchJson(apiBase, `/transactions/${encodeURIComponent(dialogHash)}`, {
      signal: detailRequestAbortController.signal,
    })
      .then((detail) => {
        if (cancelled) {
          return;
        }
        setTransactionDetailByHash(dialogHash, detail, detailCacheLimit);
        setDialogLoading(false);
      })
      .catch((error) => {
        if (cancelled || isAbortError(error)) {
          return;
        }
        setDialogLoading(false);
        setDialogError(`Failed to load tx details: ${error.message}`);
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
