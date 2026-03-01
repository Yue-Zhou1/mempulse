import { useEffect, useMemo, useRef } from 'react';
import { filterRowsByMainnet } from '../domain/mainnet-filter.js';
import { resolveDashboardRuntimePolicy } from '../domain/screen-runtime-policy.js';
import {
  buildIncrementalRowIndex,
  chainStatusTone,
  formatChainStatusChainKey,
  formatDurationToken,
  formatRelativeTime,
  formatTime,
  opportunityRowKey,
  paginationWindow,
  resolveMainnetLabel,
  sparklinePath,
} from '../lib/dashboard-helpers.js';
import {
  buildVirtualizedTickerRows,
  resolveVirtualizedSelectionIndex,
} from '../lib/radar-virtualized-model.js';

const EMPTY_ROWS = [];
const EMPTY_FEATURE_MAP = new Map();

export function useDashboardDerivedState({
  activeScreen,
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
}) {
  const previousVirtualizedTickerRowsRef = useRef(null);
  const featureByHashRef = useRef(EMPTY_FEATURE_MAP);
  const transactionByHashRef = useRef(new Map());
  const recentTxByHashRef = useRef(new Map());
  const filteredOpportunityByKeyRef = useRef(new Map());
  const runtimePolicy = useMemo(
    () => resolveDashboardRuntimePolicy(activeScreen),
    [activeScreen],
  );
  const shouldComputeRadarDerived = runtimePolicy.shouldComputeRadarDerived;

  const featureByHash = useMemo(() => {
    if (!shouldComputeRadarDerived && !dialogHash) {
      featureByHashRef.current = EMPTY_FEATURE_MAP;
      return EMPTY_FEATURE_MAP;
    }
    const previous = featureByHashRef.current === EMPTY_FEATURE_MAP
      ? null
      : featureByHashRef.current;
    const next = buildIncrementalRowIndex(previous, featureDetailRows, (row) => row?.hash);
    featureByHashRef.current = next;
    return next;
  }, [dialogHash, featureDetailRows, shouldComputeRadarDerived]);

  const transactionByHash = useMemo(() => {
    const next = buildIncrementalRowIndex(
      transactionByHashRef.current,
      transactionRows,
      (row) => row?.hash,
    );
    transactionByHashRef.current = next;
    return next;
  }, [transactionRows]);

  const recentTxByHash = useMemo(() => {
    const next = buildIncrementalRowIndex(
      recentTxByHashRef.current,
      recentTxRows,
      (row) => row?.hash,
    );
    recentTxByHashRef.current = next;
    return next;
  }, [recentTxRows]);

  const filteredTransactions = useMemo(() => {
    if (!shouldComputeRadarDerived) {
      return EMPTY_ROWS;
    }
    const scopedRows = filterRowsByMainnet(
      transactionRows,
      (row) => resolveMainnetLabel(row.chain_id, row.source_id),
      liveMainnetFilter,
    );
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return scopedRows;
    }

    return scopedRows.filter((row) => {
      const feature = featureByHash.get(row.hash);
      return (
        row.hash?.toLowerCase().includes(needle)
        || row.peer?.toLowerCase().includes(needle)
        || row.sender?.toLowerCase().includes(needle)
        || row.source_id?.toLowerCase().includes(needle)
        || resolveMainnetLabel(row.chain_id, row.source_id).toLowerCase().includes(needle)
        || String(row.tx_type ?? '').toLowerCase().includes(needle)
        || String(row.nonce ?? '').includes(needle)
        || feature?.protocol?.toLowerCase().includes(needle)
        || feature?.category?.toLowerCase().includes(needle)
      );
    });
  }, [
    featureByHash,
    liveMainnetFilter,
    query,
    shouldComputeRadarDerived,
    transactionRows,
  ]);

  const filteredOpportunityRows = useMemo(
    () => filterRowsByMainnet(
      opportunityRows,
      (row) => resolveMainnetLabel(row.chain_id, row.source_id),
      liveMainnetFilter,
    ),
    [liveMainnetFilter, opportunityRows],
  );

  const filteredOpportunityByKey = useMemo(() => {
    const next = buildIncrementalRowIndex(
      filteredOpportunityByKeyRef.current,
      filteredOpportunityRows,
      opportunityRowKey,
    );
    filteredOpportunityByKeyRef.current = next;
    return next;
  }, [filteredOpportunityRows]);

  const selectedTransaction = useMemo(
    () => (shouldComputeRadarDerived
      ? transactionByHash.get(selectedHash) ?? null
      : null),
    [selectedHash, shouldComputeRadarDerived, transactionByHash],
  );

  const selectedDetail = useMemo(() => {
    if (!selectedTransaction) {
      return null;
    }
    return resolveTransactionDetail(selectedTransaction.hash);
  }, [resolveTransactionDetail, selectedTransaction, transactionDetailVersion]);

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
    return recentTxByHash.get(selectedTransaction.hash) ?? null;
  }, [recentTxByHash, selectedTransaction]);

  const selectedOpportunity = useMemo(
    () => filteredOpportunityByKey.get(selectedOpportunityKey) ?? null,
    [filteredOpportunityByKey, selectedOpportunityKey],
  );

  const selectedTransactionMainnet = selectedTransaction
    ? resolveMainnetLabel(selectedTransaction.chain_id, selectedTransaction.source_id)
    : '-';
  const selectedOpportunityMainnet = selectedOpportunity
    ? resolveMainnetLabel(selectedOpportunity.chain_id, selectedOpportunity.source_id)
    : '-';

  const dialogTransaction = useMemo(() => {
    if (!dialogHash) {
      return null;
    }
    return transactionByHash.get(dialogHash) ?? null;
  }, [dialogHash, transactionByHash]);

  const dialogDetail = useMemo(() => {
    if (!dialogHash) {
      return null;
    }
    return resolveTransactionDetail(dialogHash);
  }, [dialogHash, resolveTransactionDetail, transactionDetailVersion]);

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
  const dialogMainnet = resolveMainnetLabel(
    dialogDetail?.chain_id ?? dialogTransaction?.chain_id,
    dialogTransaction?.source_id ?? dialogDetail?.source_id,
  );

  const maxPagedRows = Math.min(
    tickerPageSize * tickerPageLimit,
    Math.max(
      tickerPageSize,
      Number.isFinite(maxRenderedTransactions)
        ? Math.floor(maxRenderedTransactions)
        : tickerPageSize * tickerPageLimit,
    ),
  );
  const pagedTransactions = useMemo(
    () => (shouldComputeRadarDerived
      ? filteredTransactions.slice(0, maxPagedRows)
      : EMPTY_ROWS),
    [filteredTransactions, maxPagedRows, shouldComputeRadarDerived],
  );

  const transactionPageCount = Math.max(
    1,
    Math.ceil(pagedTransactions.length / tickerPageSize),
  );
  const normalizedTransactionPage = Math.min(transactionPage, transactionPageCount);
  const transactionPageStart = (normalizedTransactionPage - 1) * tickerPageSize;
  const latestTickerRows = useMemo(
    () => (shouldComputeRadarDerived
      ? pagedTransactions.slice(
        transactionPageStart,
        transactionPageStart + tickerPageSize,
      )
      : EMPTY_ROWS),
    [pagedTransactions, shouldComputeRadarDerived, tickerPageSize, transactionPageStart],
  );
  const deferredTickerRows = latestTickerRows;
  const virtualizedTickerRows = useMemo(
    () => {
      const next = buildVirtualizedTickerRows(
        deferredTickerRows,
        tickerPageSize,
        previousVirtualizedTickerRowsRef.current,
      );
      previousVirtualizedTickerRowsRef.current = next;
      return next;
    },
    [deferredTickerRows, tickerPageSize],
  );
  const virtualizedSelectedTickerIndex = useMemo(
    () => resolveVirtualizedSelectionIndex(virtualizedTickerRows, selectedHash),
    [selectedHash, virtualizedTickerRows],
  );
  const transactionPageEnd = Math.min(
    pagedTransactions.length,
    transactionPageStart + latestTickerRows.length,
  );
  const paginationPages = useMemo(
    () => paginationWindow(normalizedTransactionPage, transactionPageCount),
    [normalizedTransactionPage, transactionPageCount],
  );

  useEffect(() => {
    setTransactionPage((current) => Math.min(current, transactionPageCount));
  }, [setTransactionPage, transactionPageCount]);

  useEffect(() => {
    setSelectedOpportunityKey((current) => {
      if (current && filteredOpportunityByKey.has(current)) {
        return current;
      }
      return filteredOpportunityRows[0] ? opportunityRowKey(filteredOpportunityRows[0]) : null;
    });
  }, [filteredOpportunityByKey, filteredOpportunityRows, setSelectedOpportunityKey]);

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

  const chainStatusBadges = useMemo(
    () => chainStatusRows.map((row) => {
      const displayState = row.state.replaceAll('_', ' ');
      const silentToken = row.silent_for_ms == null ? '-' : formatDurationToken(row.silent_for_ms);
      const endpointToken = row.endpoint_count > 0
        ? `${Math.min(row.endpoint_index + 1, row.endpoint_count)}/${row.endpoint_count}`
        : '-/-';
      const titleParts = [
        `${row.chain_key} (${row.source_id})`,
        `state=${displayState}`,
        `endpoint=${endpointToken}`,
        row.ws_url ? `ws=${row.ws_url}` : '',
        row.last_error ? `error=${row.last_error}` : '',
      ].filter(Boolean);
      return {
        key: row.chain_key,
        chainLabel: formatChainStatusChainKey(row.chain_key),
        stateLabel: displayState,
        silentToken,
        endpointToken,
        rotations: row.rotation_count,
        tone: chainStatusTone(row.state),
        title: titleParts.join('\n'),
      };
    }),
    [chainStatusRows],
  );

  return {
    featureByHash,
    filteredTransactions,
    filteredOpportunityRows,
    selectedTransaction,
    selectedDetail,
    selectedFeature,
    selectedRecent,
    selectedOpportunity,
    selectedTransactionMainnet,
    selectedOpportunityMainnet,
    dialogTransaction,
    dialogDetail,
    dialogFeature,
    detailSender,
    detailSeenAt,
    detailSeenRelative,
    dialogMainnet,
    pagedTransactions,
    transactionPageCount,
    normalizedTransactionPage,
    transactionPageStart,
    deferredTickerRows,
    virtualizedTickerRows,
    virtualizedSelectedTickerIndex,
    transactionPageEnd,
    paginationPages,
    totalSignalVolume,
    totalTxCount,
    lowRiskCount,
    mediumRiskCount,
    highRiskCount,
    successRate,
    featureTrendPath,
    topMixRows,
    mixTotal,
    chainStatusBadges,
  };
}
