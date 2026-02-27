import { useCallback, useMemo, useRef, useState } from 'react';
import { resolveApiBase } from '../../../network/api-base.js';
import { readWindowRuntimeConfig, resolveUiRuntimeConfig } from '../domain/ui-config.js';
import {
  getTransactionDetailByHash,
  useDashboardStreamStore,
} from '../domain/dashboard-stream-store.js';
import { createMarketStatsState } from '../domain/market-stats.js';
import { normalizeScreenId } from '../domain/screen-mode.js';
import { createTxLiveStore } from '../domain/tx-live-store.js';
import { MAINNET_FILTER_ALL } from '../domain/mainnet-filter.js';
import { getStoredApiBase, setStoredApiBase } from '../lib/dashboard-helpers.js';
import { useDashboardActions } from './use-dashboard-actions.js';
import { useDashboardDerivedState } from './use-dashboard-derived-state.js';
import { useDashboardLifecycleEffects } from './use-dashboard-lifecycle-effects.js';
import { useSelectedTransactionDetail } from './use-selected-transaction-detail.js';

const tickerPageSize = 50;
const tickerPageLimit = 10;

export function useDashboardController() {
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
    () => resolveUiRuntimeConfig({
      runtime: readWindowRuntimeConfig(),
      env: import.meta.env,
    }),
    [],
  );
  const {
    snapshotTxLimit,
    detailCacheLimit,
    maxTransactionHistory,
    maxFeatureHistory,
    maxOpportunityHistory,
    maxRenderedTransactions,
    streamBatchMs,
    samplingLagThresholdMs,
    samplingStride,
    samplingFlushIdleMs,
    heapEmergencyPurgeMb,
    transactionRetentionMs,
    transactionRetentionMinutes,
    workerEnabled,
    virtualizedTickerEnabled,
  } = uiConfig;

  const [statusMessage, setStatusMessage] = useState('Connecting to API...');
  const [hasError, setHasError] = useState(false);
  const [activeScreen, setActiveScreen] = useState(() =>
    normalizeScreenId(window.location.hash.replace(/^#/, '')),
  );
  const [query, setQuery] = useState('');
  const [chainStatusRows, setChainStatusRows] = useState([]);
  const [liveMainnetFilter, setLiveMainnetFilter] = useState(MAINNET_FILTER_ALL);
  const [showTickerFilters, setShowTickerFilters] = useState(false);
  const [transactionPage, setTransactionPage] = useState(1);
  const [followLatest, setFollowLatest] = useState(true);
  const [selectedOpportunityKey, setSelectedOpportunityKey] = useState(null);
  const [dialogHash, setDialogHash] = useState(null);
  const [dialogError, setDialogError] = useState('');
  const [dialogLoading, setDialogLoading] = useState(false);

  const [opportunityRows, setOpportunityRows] = useState([]);
  const [featureSummaryRows, setFeatureSummaryRows] = useState([]);
  const [featureDetailRows, setFeatureDetailRows] = useState([]);
  const [marketStats, setMarketStats] = useState(() => createMarketStatsState());

  const recentTxRows = useDashboardStreamStore((state) => state.recentTxRows);
  const transactionRows = useDashboardStreamStore((state) => state.transactionRows);
  const selectedHash = useDashboardStreamStore((state) => state.activeSelectedTxId);
  const transactionDetailVersion = useDashboardStreamStore((state) => state.detailVersion);
  const enqueueTickerCommit = useDashboardStreamStore((state) => state.enqueueTickerCommit);
  const flushTickerCommit = useDashboardStreamStore((state) => state.flushTickerCommit);
  const cancelTickerCommit = useDashboardStreamStore((state) => state.cancelTickerCommit);
  const setActiveSelectedTxId = useDashboardStreamStore((state) => state.setActiveSelectedTxId);

  const transactionRowsRef = useRef([]);
  const recentTxRowsRef = useRef([]);
  const transactionStoreRef = useRef(createTxLiveStore());
  const followLatestRef = useRef(followLatest);
  const latestSeqIdRef = useRef(0);
  const lastGapSnapshotAtRef = useRef(0);
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef(null);
  const snapshotTimerRef = useRef(null);
  const snapshotInFlightRef = useRef(false);
  const nextSnapshotAtRef = useRef(0);

  const resolveTransactionDetail = useCallback(
    (hash) => getTransactionDetailByHash(hash),
    [],
  );
  const selectedDetail = useSelectedTransactionDetail(selectedHash);

  const derived = useDashboardDerivedState({
    transactionRows,
    liveMainnetFilter,
    query,
    featureDetailRows,
    opportunityRows,
    selectedHash,
    resolveTransactionDetail,
    transactionDetailVersion,
    recentTxRows,
    selectedOpportunityKey,
    dialogHash,
    transactionPage,
    tickerPageSize,
    tickerPageLimit,
    maxRenderedTransactions,
    chainStatusRows,
    marketStats,
    featureSummaryRows,
    setTransactionPage,
    setSelectedOpportunityKey,
  });

  const actions = useDashboardActions({
    setActiveScreen,
    setQuery,
    setShowTickerFilters,
    setLiveMainnetFilter,
    setFollowLatest,
    setTransactionPage,
    transactionPageCount: derived.transactionPageCount,
    setSelectedHash: setActiveSelectedTxId,
    setDialogHash,
    setDialogError,
    setSelectedOpportunityKey,
  });

  useDashboardLifecycleEffects({
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
    closeDialog: actions.closeDialog,
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
  });

  const editionDate = new Date().toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
    year: 'numeric',
  });

  const model = {
    editionDate,
    activeScreen,
    statusMessage,
    hasError,
    virtualizedTickerEnabled,
    chainStatusBadges: derived.chainStatusBadges,
    showTickerFilters,
    liveMainnetFilter,
    query,
    deferredTickerRows: derived.deferredTickerRows,
    virtualizedTickerRows: derived.virtualizedTickerRows,
    virtualizedSelectedTickerIndex: derived.virtualizedSelectedTickerIndex,
    featureByHash: derived.featureByHash,
    selectedHash,
    filteredTransactions: derived.filteredTransactions,
    transactionPageStart: derived.transactionPageStart,
    transactionPageEnd: derived.transactionPageEnd,
    pagedTransactions: derived.pagedTransactions,
    normalizedTransactionPage: derived.normalizedTransactionPage,
    transactionPageCount: derived.transactionPageCount,
    paginationPages: derived.paginationPages,
    selectedTransaction: derived.selectedTransaction,
    selectedDetail,
    selectedFeature: derived.selectedFeature,
    selectedRecent: derived.selectedRecent,
    selectedTransactionMainnet: derived.selectedTransactionMainnet,
    totalSignalVolume: derived.totalSignalVolume,
    successRate: derived.successRate,
    totalTxCount: derived.totalTxCount,
    highRiskCount: derived.highRiskCount,
    featureTrendPath: derived.featureTrendPath,
    lowRiskCount: derived.lowRiskCount,
    mediumRiskCount: derived.mediumRiskCount,
    topMixRows: derived.topMixRows,
    mixTotal: derived.mixTotal,
    filteredOpportunityRows: derived.filteredOpportunityRows,
    selectedOpportunityKey,
    selectedOpportunity: derived.selectedOpportunity,
    selectedOpportunityMainnet: derived.selectedOpportunityMainnet,
    dialogHash,
    dialogLoading,
    dialogTransaction: derived.dialogTransaction,
    dialogMainnet: derived.dialogMainnet,
    detailSeenRelative: derived.detailSeenRelative,
    detailSender: derived.detailSender,
    detailSeenAt: derived.detailSeenAt,
    dialogFeature: derived.dialogFeature,
    dialogDetail: derived.dialogDetail,
    dialogError,
  };

  return { model, actions };
}
