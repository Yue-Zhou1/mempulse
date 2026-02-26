import { useEffect } from 'react';
import { resolveMarketStatsSnapshot } from '../domain/market-stats.js';
import { normalizeScreenId } from '../domain/screen-mode.js';
import {
  buildDashboardSnapshotPath,
  resolveDashboardSnapshotLimits,
} from '../domain/snapshot-profile.js';
import {
  fetchJson,
  normalizeChainStatusRows,
  opportunityRowKey,
} from '../lib/dashboard-helpers.js';

const snapshotThrottleMs = 1200;
const transactionCommitBatchMs = 200;
const streamBatchLimit = 512;
const streamIntervalMs = 200;
const streamInitialReconnectMs = 1000;
const streamMaxReconnectMs = 30000;
const archiveRefreshMs = 15000;

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

function replayPointEquivalent(left, right) {
  return (
    left.seq_hi === right.seq_hi
    && left.timestamp_unix_ms === right.timestamp_unix_ms
    && left.pending_count === right.pending_count
  );
}

function propagationEdgeEquivalent(left, right) {
  return (
    left.source === right.source
    && left.destination === right.destination
    && left.p50_delay_ms === right.p50_delay_ms
    && left.p99_delay_ms === right.p99_delay_ms
  );
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

export function useDashboardLifecycleEffects({
  apiBase,
  snapshotTxLimit,
  maxTransactionHistory,
  transactionRetentionMs,
  transactionRetentionMinutes,
  detailCacheLimit,
  activeScreen,
  refreshArchives,
  dialogHash,
  transactionDetailsByHash,
  closeDialog,
  followLatest,
  recentTxRows,
  transactionRows,
  recentTxRowsRef,
  transactionRowsRef,
  transactionStoreRef,
  followLatestRef,
  latestSeqIdRef,
  reconnectAttemptsRef,
  reconnectTimerRef,
  snapshotTimerRef,
  streamSocketRef,
  snapshotInFlightRef,
  nextSnapshotAtRef,
  setTransactionPage,
  setActiveScreen,
  setReplayFrames,
  setPropagationEdges,
  setOpportunityRows,
  setFeatureSummaryRows,
  setFeatureDetailRows,
  setChainStatusRows,
  setMarketStats,
  setRecentTxRows,
  setTransactionRows,
  setTransactionDetailsByHash,
  setTimelineIndex,
  setSelectedHash,
  setSelectedOpportunityKey,
  setStatusMessage,
  setHasError,
  setDialogLoading,
  setDialogError,
}) {
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
    const snapshotPath = buildDashboardSnapshotPath(
      resolveDashboardSnapshotLimits({ activeScreen, snapshotTxLimit }),
    );
    let pendingTransactionCommit = null;
    let transactionCommitTimer = null;

    const cleanupSocket = () => {
      if (streamSocketRef.current) {
        const socket = streamSocketRef.current;
        streamSocketRef.current = null;
        socket.onmessage = null;
        socket.onclose = null;
        socket.onerror = null;

        if (socket.readyState === WebSocket.CONNECTING) {
          socket.onopen = () => {
            socket.onopen = null;
            socket.close();
          };
          return;
        }

        socket.onopen = null;
        if (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CLOSING) {
          socket.close();
        }
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

    const clearTransactionCommitTimer = () => {
      if (transactionCommitTimer) {
        window.clearTimeout(transactionCommitTimer);
        transactionCommitTimer = null;
      }
    };

    const flushTransactionCommit = () => {
      transactionCommitTimer = null;
      if (cancelled || !pendingTransactionCommit) {
        return;
      }
      const commit = pendingTransactionCommit;
      pendingTransactionCommit = null;

      setRecentTxRows(commit.recentTxRows);
      setTransactionRows(commit.transactionRows);
      setTransactionDetailsByHash((current) => {
        const liveHashes = new Set(commit.transactionRows.map((row) => row.hash));
        let changed = false;
        const next = {};
        for (const [hash, detail] of Object.entries(current)) {
          if (liveHashes.has(hash)) {
            next[hash] = detail;
          } else {
            changed = true;
          }
        }
        if (!changed && Object.keys(next).length === Object.keys(current).length) {
          return current;
        }
        return next;
      });
      setSelectedHash((current) => {
        if (current && commit.transactionRows.some((row) => row.hash === current)) {
          return current;
        }
        return commit.transactionRows[0]?.hash ?? null;
      });
    };

    const queueTransactionCommit = (commit) => {
      pendingTransactionCommit = commit;
      if (transactionCommitTimer) {
        return;
      }
      transactionCommitTimer = window.setTimeout(() => {
        flushTransactionCommit();
      }, transactionCommitBatchMs);
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
      setReplayFrames((current) =>
        stabilizeRows(current, replay, (row) => row.seq_hi, replayPointEquivalent),
      );
      setPropagationEdges((current) =>
        stabilizeRows(
          current,
          propagation,
          (row) => `${row.source}->${row.destination}`,
          propagationEdgeEquivalent,
        ),
      );
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

      setTimelineIndex((current) => {
        if (!replay.length) {
          return 0;
        }
        if (followLatestRef.current) {
          return replay.length - 1;
        }
        return Math.min(current, replay.length - 1);
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
      flushTransactionCommit();
      clearTransactionCommitTimer();
      cancelled = true;
      clearReconnectTimer();
      clearSnapshotTimer();
      cleanupSocket();
    };
  }, [
    apiBase,
    activeScreen,
    followLatestRef,
    latestSeqIdRef,
    maxTransactionHistory,
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
    setPropagationEdges,
    setRecentTxRows,
    setReplayFrames,
    setSelectedHash,
    setSelectedOpportunityKey,
    setStatusMessage,
    setTimelineIndex,
    setTransactionRows,
    setTransactionDetailsByHash,
    snapshotInFlightRef,
    snapshotTimerRef,
    snapshotTxLimit,
    streamSocketRef,
    transactionStoreRef,
    transactionRetentionMinutes,
    transactionRetentionMs,
    transactionRowsRef,
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
  }, [
    apiBase,
    detailCacheLimit,
    dialogHash,
    setDialogError,
    setDialogLoading,
    setTransactionDetailsByHash,
    transactionDetailsByHash,
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
