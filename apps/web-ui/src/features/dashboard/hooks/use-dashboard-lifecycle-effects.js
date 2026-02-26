import { useEffect } from 'react';
import { resolveMarketStatsSnapshot } from '../domain/market-stats.js';
import { normalizeScreenId } from '../domain/screen-mode.js';
import { mergeTransactionHistory } from '../domain/tx-history.js';
import {
  fetchJson,
  normalizeChainStatusRows,
  opportunityRowKey,
} from '../lib/dashboard-helpers.js';

const snapshotFeatureLimit = 600;
const snapshotOppLimit = 600;
const snapshotReplayLimit = 1000;
const snapshotThrottleMs = 1200;
const streamBatchLimit = 512;
const streamIntervalMs = 200;
const streamInitialReconnectMs = 1000;
const streamMaxReconnectMs = 30000;
const archiveRefreshMs = 15000;

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
  transactionRows,
  query,
  liveMainnetFilter,
  transactionRowsRef,
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
      const chainStatus = normalizeChainStatusRows(snapshot?.chain_ingest_status);

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
      setChainStatusRows(chainStatus);
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
    followLatestRef,
    latestSeqIdRef,
    maxTransactionHistory,
    nextSnapshotAtRef,
    reconnectAttemptsRef,
    reconnectTimerRef,
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
