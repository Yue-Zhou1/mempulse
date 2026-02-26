import { useDeferredValue, useEffect, useMemo } from 'react';
import { filterRowsByMainnet } from '../domain/mainnet-filter.js';
import {
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

export function useDashboardDerivedState({
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
}) {
  const featureByHash = useMemo(() => {
    const map = new Map();
    for (const row of featureDetailRows) {
      map.set(row.hash, row);
    }
    return map;
  }, [featureDetailRows]);

  const filteredTransactions = useMemo(() => {
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
        row.hash?.toLowerCase().includes(needle) ||
        row.peer?.toLowerCase().includes(needle) ||
        row.sender?.toLowerCase().includes(needle) ||
        row.source_id?.toLowerCase().includes(needle) ||
        resolveMainnetLabel(row.chain_id, row.source_id).toLowerCase().includes(needle) ||
        String(row.tx_type ?? '').toLowerCase().includes(needle) ||
        String(row.nonce ?? '').includes(needle) ||
        feature?.protocol?.toLowerCase().includes(needle) ||
        feature?.category?.toLowerCase().includes(needle)
      );
    });
  }, [featureByHash, liveMainnetFilter, query, transactionRows]);

  const filteredOpportunityRows = useMemo(
    () =>
      filterRowsByMainnet(
        opportunityRows,
        (row) => resolveMainnetLabel(row.chain_id, row.source_id),
        liveMainnetFilter,
      ),
    [liveMainnetFilter, opportunityRows],
  );

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
    () => filteredOpportunityRows.find((row) => opportunityRowKey(row) === selectedOpportunityKey) ?? null,
    [filteredOpportunityRows, selectedOpportunityKey],
  );

  const selectedArchiveTx = useMemo(
    () => archiveTxRows.find((row) => row.hash === selectedArchiveTxHash) ?? null,
    [archiveTxRows, selectedArchiveTxHash],
  );

  const selectedArchiveOpp = useMemo(
    () => archiveOppRows.find((row) => opportunityRowKey(row) === selectedArchiveOppKey) ?? null,
    [archiveOppRows, selectedArchiveOppKey],
  );

  const selectedTransactionMainnet = selectedTransaction
    ? resolveMainnetLabel(selectedTransaction.chain_id, selectedTransaction.source_id)
    : '-';
  const selectedOpportunityMainnet = selectedOpportunity
    ? resolveMainnetLabel(selectedOpportunity.chain_id, selectedOpportunity.source_id)
    : '-';
  const selectedArchiveTxMainnet = selectedArchiveTx
    ? resolveMainnetLabel(selectedArchiveTx.chain_id, selectedArchiveTx.source_id)
    : '-';
  const selectedArchiveOppMainnet = selectedArchiveOpp
    ? resolveMainnetLabel(selectedArchiveOpp.chain_id, selectedArchiveOpp.source_id)
    : '-';

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
  const dialogMainnet = resolveMainnetLabel(
    dialogDetail?.chain_id ?? dialogTransaction?.chain_id,
    dialogTransaction?.source_id ?? dialogDetail?.source_id,
  );

  const maxPagedRows = tickerPageSize * tickerPageLimit;
  const pagedTransactions = useMemo(
    () => filteredTransactions.slice(0, maxPagedRows),
    [filteredTransactions, maxPagedRows],
  );

  const transactionPageCount = Math.max(
    1,
    Math.ceil(pagedTransactions.length / tickerPageSize),
  );
  const normalizedTransactionPage = Math.min(transactionPage, transactionPageCount);
  const transactionPageStart = (normalizedTransactionPage - 1) * tickerPageSize;
  const latestTickerRows = pagedTransactions.slice(
    transactionPageStart,
    transactionPageStart + tickerPageSize,
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
    const scopedRows = filterRowsByMainnet(
      archiveTxRows,
      (row) => resolveMainnetLabel(row.chain_id, row.source_id),
      archiveMainnetFilter,
    );
    if (!archiveNeedle) {
      return scopedRows;
    }
    return scopedRows.filter((row) => (
      row.hash?.toLowerCase().includes(archiveNeedle) ||
      row.sender?.toLowerCase().includes(archiveNeedle) ||
      row.peer?.toLowerCase().includes(archiveNeedle) ||
      resolveMainnetLabel(row.chain_id, row.source_id).toLowerCase().includes(archiveNeedle) ||
      row.protocol?.toLowerCase().includes(archiveNeedle) ||
      row.category?.toLowerCase().includes(archiveNeedle) ||
      row.lifecycle_status?.toLowerCase().includes(archiveNeedle) ||
      String(row.nonce ?? '').includes(archiveNeedle)
    ));
  }, [archiveMainnetFilter, archiveNeedle, archiveTxRows]);

  const filteredArchiveOppRows = useMemo(() => {
    const scopedRows = filterRowsByMainnet(
      archiveOppRows,
      (row) => resolveMainnetLabel(row.chain_id, row.source_id),
      archiveMainnetFilter,
    );
    if (!archiveNeedle) {
      return scopedRows;
    }
    return scopedRows.filter((row) => (
      row.tx_hash?.toLowerCase().includes(archiveNeedle) ||
      row.strategy?.toLowerCase().includes(archiveNeedle) ||
      resolveMainnetLabel(row.chain_id, row.source_id).toLowerCase().includes(archiveNeedle) ||
      row.protocol?.toLowerCase().includes(archiveNeedle) ||
      row.category?.toLowerCase().includes(archiveNeedle) ||
      row.status?.toLowerCase().includes(archiveNeedle) ||
      row.reasons?.some((reason) => reason?.toLowerCase().includes(archiveNeedle))
    ));
  }, [archiveMainnetFilter, archiveNeedle, archiveOppRows]);

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
  }, [setTransactionPage, transactionPageCount]);

  useEffect(() => {
    setArchiveTxPage((current) => Math.min(current, archiveTxPageCount));
  }, [archiveTxPageCount, setArchiveTxPage]);

  useEffect(() => {
    setArchiveOppPage((current) => Math.min(current, archiveOppPageCount));
  }, [archiveOppPageCount, setArchiveOppPage]);

  useEffect(() => {
    setArchiveTxPage(1);
    setArchiveOppPage(1);
  }, [archiveMainnetFilter, archiveQuery, setArchiveOppPage, setArchiveTxPage]);

  useEffect(() => {
    setSelectedOpportunityKey((current) => {
      if (current && filteredOpportunityRows.some((row) => opportunityRowKey(row) === current)) {
        return current;
      }
      return filteredOpportunityRows[0] ? opportunityRowKey(filteredOpportunityRows[0]) : null;
    });
  }, [filteredOpportunityRows, setSelectedOpportunityKey]);

  useEffect(() => {
    setSelectedArchiveTxHash((current) => {
      if (current && filteredArchiveTxRows.some((row) => row.hash === current)) {
        return current;
      }
      return filteredArchiveTxRows[0]?.hash ?? null;
    });
  }, [filteredArchiveTxRows, setSelectedArchiveTxHash]);

  useEffect(() => {
    setSelectedArchiveOppKey((current) => {
      if (current && filteredArchiveOppRows.some((row) => opportunityRowKey(row) === current)) {
        return current;
      }
      return filteredArchiveOppRows[0] ? opportunityRowKey(filteredArchiveOppRows[0]) : null;
    });
  }, [filteredArchiveOppRows, setSelectedArchiveOppKey]);

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
    () =>
      chainStatusRows.map((row) => {
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
    selectedArchiveTx,
    selectedArchiveOpp,
    selectedTransactionMainnet,
    selectedOpportunityMainnet,
    selectedArchiveTxMainnet,
    selectedArchiveOppMainnet,
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
    transactionPageEnd,
    paginationPages,
    filteredArchiveTxRows,
    filteredArchiveOppRows,
    archiveTxPageCount,
    normalizedArchiveTxPage,
    archiveTxPageRows,
    archiveTxPages,
    archiveOppPageCount,
    normalizedArchiveOppPage,
    archiveOppPageRows,
    archiveOppPages,
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
