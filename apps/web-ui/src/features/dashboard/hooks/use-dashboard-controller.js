import { useMemo, useRef, useState } from 'react';
import { resolveApiBase } from '../../../network/api-base.js';
import { readWindowRuntimeConfig, resolveUiRuntimeConfig } from '../domain/ui-config.js';
import { createMarketStatsState } from '../domain/market-stats.js';
import { normalizeScreenId } from '../domain/screen-mode.js';
import { MAINNET_FILTER_ALL } from '../domain/mainnet-filter.js';
import { getStoredApiBase, setStoredApiBase } from '../lib/dashboard-helpers.js';
import { useArchiveRefresh } from './use-archive-refresh.js';
import { useDashboardActions } from './use-dashboard-actions.js';
import { useDashboardDerivedState } from './use-dashboard-derived-state.js';
import { useDashboardLifecycleEffects } from './use-dashboard-lifecycle-effects.js';

const tickerPageSize = 50;
const tickerPageLimit = 10;
const archiveTxPageSize = 40;
const archiveOppPageSize = 20;

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
  const [chainStatusRows, setChainStatusRows] = useState([]);
  const [liveMainnetFilter, setLiveMainnetFilter] = useState(MAINNET_FILTER_ALL);
  const [archiveMainnetFilter, setArchiveMainnetFilter] = useState(MAINNET_FILTER_ALL);
  const [showTickerFilters, setShowTickerFilters] = useState(false);
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

  const refreshArchives = useArchiveRefresh({
    apiBase,
    archiveRequestSeqRef,
    setArchiveLoading,
    setArchiveError,
    setArchiveTxRows,
    setArchiveOppRows,
    setSelectedArchiveTxHash,
    setSelectedArchiveOppKey,
  });

  const derived = useDashboardDerivedState({
    transactionRows,
    liveMainnetFilter,
    query,
    featureDetailRows,
    opportunityRows,
    selectedHash,
    transactionDetailsByHash,
    recentTxRows,
    selectedOpportunityKey,
    archiveTxRows,
    selectedArchiveTxHash,
    archiveOppRows,
    selectedArchiveOppKey,
    dialogHash,
    transactionPage,
    tickerPageSize,
    tickerPageLimit,
    archiveQuery,
    archiveMainnetFilter,
    archiveTxPage,
    archiveTxPageSize,
    archiveOppPage,
    archiveOppPageSize,
    chainStatusRows,
    marketStats,
    featureSummaryRows,
    setTransactionPage,
    setArchiveTxPage,
    setArchiveOppPage,
    setSelectedOpportunityKey,
    setSelectedArchiveTxHash,
    setSelectedArchiveOppKey,
  });

  const actions = useDashboardActions({
    setActiveScreen,
    setQuery,
    setShowTickerFilters,
    setLiveMainnetFilter,
    setFollowLatest,
    setTransactionPage,
    transactionPageCount: derived.transactionPageCount,
    replayFrames,
    setTimelineIndex,
    setSelectedHash,
    setDialogHash,
    setDialogError,
    setSelectedOpportunityKey,
    setSelectedArchiveTxHash,
    setSelectedArchiveOppKey,
    setArchiveQuery,
    setArchiveMainnetFilter,
    setArchiveTxPage,
    archiveTxPageCount: derived.archiveTxPageCount,
    setArchiveOppPage,
    archiveOppPageCount: derived.archiveOppPageCount,
    refreshArchives,
  });

  useDashboardLifecycleEffects({
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
    closeDialog: actions.closeDialog,
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
    chainStatusBadges: derived.chainStatusBadges,
    showTickerFilters,
    liveMainnetFilter,
    query,
    deferredTickerRows: derived.deferredTickerRows,
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
    selectedFeature: derived.selectedFeature,
    selectedRecent: derived.selectedRecent,
    selectedTransactionMainnet: derived.selectedTransactionMainnet,
    totalSignalVolume: derived.totalSignalVolume,
    successRate: derived.successRate,
    totalTxCount: derived.totalTxCount,
    replayFrames,
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
    archiveLoading,
    archiveQuery,
    archiveMainnetFilter,
    filteredArchiveTxRows: derived.filteredArchiveTxRows,
    archiveTxRows,
    filteredArchiveOppRows: derived.filteredArchiveOppRows,
    archiveOppRows,
    archiveError,
    archiveTxPageRows: derived.archiveTxPageRows,
    selectedArchiveTxHash,
    normalizedArchiveTxPage: derived.normalizedArchiveTxPage,
    archiveTxPages: derived.archiveTxPages,
    archiveTxPageCount: derived.archiveTxPageCount,
    archiveOppPageRows: derived.archiveOppPageRows,
    selectedArchiveOppKey,
    normalizedArchiveOppPage: derived.normalizedArchiveOppPage,
    archiveOppPages: derived.archiveOppPages,
    archiveOppPageCount: derived.archiveOppPageCount,
    selectedArchiveTx: derived.selectedArchiveTx,
    selectedArchiveTxMainnet: derived.selectedArchiveTxMainnet,
    selectedArchiveOpp: derived.selectedArchiveOpp,
    selectedArchiveOppMainnet: derived.selectedArchiveOppMainnet,
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
