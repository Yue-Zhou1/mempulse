import { useCallback } from 'react';
import { normalizeMainnetFilter } from '../domain/mainnet-filter.js';

export function useDashboardActions({
  setActiveScreen,
  setQuery,
  setShowTickerFilters,
  setLiveMainnetFilter,
  setFollowLatest,
  setTransactionPage,
  transactionPageCount,
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
  archiveTxPageCount,
  setArchiveOppPage,
  archiveOppPageCount,
  refreshArchives,
}) {
  const closeDialog = useCallback(() => {
    setDialogHash(null);
    setDialogError('');
  }, [setDialogError, setDialogHash]);

  const openTransactionByHash = useCallback((hash) => {
    if (!hash) {
      return;
    }
    setSelectedHash(hash);
    setDialogHash(hash);
    setDialogError('');
  }, [setDialogError, setDialogHash, setSelectedHash]);

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

  const onOpportunityListClick = useCallback((event) => {
    const node = event.target;
    if (!(node instanceof Element)) {
      return;
    }
    const rowButton = node.closest('button[data-opportunity-key]');
    const key = rowButton?.getAttribute('data-opportunity-key');
    if (!key) {
      return;
    }
    setSelectedOpportunityKey(key);
  }, [setSelectedOpportunityKey]);

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
  }, [setSelectedArchiveTxHash]);

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
  }, [setSelectedArchiveOppKey]);

  const onShowRadar = useCallback(() => {
    setActiveScreen('radar');
  }, [setActiveScreen]);

  const onShowOpps = useCallback(() => {
    setActiveScreen('opps');
  }, [setActiveScreen]);

  const onShowReplay = useCallback(() => {
    setActiveScreen('replay');
  }, [setActiveScreen]);

  const onSearchChange = useCallback((event) => {
    setQuery(event.target.value);
  }, [setQuery]);

  const onTickerFilterToggle = useCallback(() => {
    setShowTickerFilters((current) => !current);
  }, [setShowTickerFilters]);

  const onLiveMainnetFilterChange = useCallback((event) => {
    setLiveMainnetFilter(normalizeMainnetFilter(event.target.value));
  }, [setLiveMainnetFilter]);

  const onTickerFollowClick = useCallback(() => {
    setFollowLatest(true);
    setTransactionPage(1);
    if (replayFrames.length) {
      setTimelineIndex(replayFrames.length - 1);
    }
  }, [replayFrames, setFollowLatest, setTimelineIndex, setTransactionPage]);

  const onTransactionPagePrev = useCallback(() => {
    setFollowLatest(false);
    setTransactionPage((current) => Math.max(1, current - 1));
  }, [setFollowLatest, setTransactionPage]);

  const onTransactionPageNext = useCallback(() => {
    setFollowLatest(false);
    setTransactionPage((current) => Math.min(transactionPageCount, current + 1));
  }, [setFollowLatest, setTransactionPage, transactionPageCount]);

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
  }, [setFollowLatest, setTransactionPage, transactionPageCount]);

  const onArchiveQueryChange = useCallback((event) => {
    setArchiveQuery(event.target.value);
  }, [setArchiveQuery]);

  const onArchiveMainnetFilterChange = useCallback((event) => {
    setArchiveMainnetFilter(normalizeMainnetFilter(event.target.value));
  }, [setArchiveMainnetFilter]);

  const onArchiveTxPagePrev = useCallback(() => {
    setArchiveTxPage((current) => Math.max(1, current - 1));
  }, [setArchiveTxPage]);

  const onArchiveTxPageNext = useCallback(() => {
    setArchiveTxPage((current) => Math.min(archiveTxPageCount, current + 1));
  }, [archiveTxPageCount, setArchiveTxPage]);

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
  }, [archiveTxPageCount, setArchiveTxPage]);

  const onArchiveOppPagePrev = useCallback(() => {
    setArchiveOppPage((current) => Math.max(1, current - 1));
  }, [setArchiveOppPage]);

  const onArchiveOppPageNext = useCallback(() => {
    setArchiveOppPage((current) => Math.min(archiveOppPageCount, current + 1));
  }, [archiveOppPageCount, setArchiveOppPage]);

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
  }, [archiveOppPageCount, setArchiveOppPage]);

  return {
    closeDialog,
    openTransactionByHash,
    onTickerListClick,
    onOpportunityListClick,
    onArchiveTxListClick,
    onArchiveOppListClick,
    onShowRadar,
    onShowOpps,
    onShowReplay,
    onSearchChange,
    onTickerFilterToggle,
    onLiveMainnetFilterChange,
    onTickerFollowClick,
    onTransactionPagePrev,
    onTransactionPageNext,
    onTransactionPaginationClick,
    onArchiveQueryChange,
    onArchiveMainnetFilterChange,
    onArchiveTxPagePrev,
    onArchiveTxPageNext,
    onArchiveTxPaginationClick,
    onArchiveOppPagePrev,
    onArchiveOppPageNext,
    onArchiveOppPaginationClick,
    refreshArchives,
  };
}
