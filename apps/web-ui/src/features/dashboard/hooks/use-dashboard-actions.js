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
  setSelectedHash,
  setDialogHash,
  setDialogError,
  setSelectedOpportunityKey,
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
      const rowElement = node.closest('[data-tx-hash]');
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
    const container = event.currentTarget;
    if (!(node instanceof Element) || !(container instanceof Element)) {
      return;
    }
    const rowElement = node.closest('[data-opportunity-key]');
    if (!rowElement || !container.contains(rowElement)) {
      return;
    }
    const key = rowElement.getAttribute('data-opportunity-key');
    if (!key) {
      return;
    }
    setSelectedOpportunityKey(key);
  }, [setSelectedOpportunityKey]);

  const onShowRadar = useCallback(() => {
    setActiveScreen('radar');
  }, [setActiveScreen]);

  const onShowOpps = useCallback(() => {
    setActiveScreen('opps');
  }, [setActiveScreen]);

  const onMastheadNavClick = useCallback((event) => {
    const node = event.target;
    const container = event.currentTarget;
    if (!(node instanceof Element) || !(container instanceof Element)) {
      return;
    }
    const tab = node.closest('[data-screen-id]');
    if (!tab || !container.contains(tab)) {
      return;
    }
    const screenId = tab.getAttribute('data-screen-id');
    if (!screenId) {
      return;
    }
    setActiveScreen(screenId);
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
  }, [setFollowLatest, setTransactionPage]);

  const onTickerToolbarClick = useCallback((event) => {
    const node = event.target;
    const container = event.currentTarget;
    if (!(node instanceof Element) || !(container instanceof Element)) {
      return;
    }
    const actionNode = node.closest('[data-ticker-action]');
    if (!actionNode || !container.contains(actionNode)) {
      return;
    }
    const action = actionNode.getAttribute('data-ticker-action');
    if (action === 'filter') {
      onTickerFilterToggle();
      return;
    }
    if (action === 'follow') {
      onTickerFollowClick();
    }
  }, [onTickerFilterToggle, onTickerFollowClick]);

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
    const container = event.currentTarget;
    if (!(node instanceof Element) || !(container instanceof Element)) {
      return;
    }
    const control = node.closest('[data-page],[data-page-action]');
    if (!control || !container.contains(control)) {
      return;
    }
    if (control.getAttribute('data-disabled') === 'true') {
      return;
    }
    const pageAction = control.getAttribute('data-page-action');
    if (pageAction === 'prev') {
      onTransactionPagePrev();
      return;
    }
    if (pageAction === 'next') {
      onTransactionPageNext();
      return;
    }
    const pageText = control.getAttribute('data-page');
    const page = Number(pageText);
    if (!Number.isFinite(page)) {
      return;
    }
    setTransactionPage(Math.max(1, Math.min(transactionPageCount, page)));
    if (page !== 1) {
      setFollowLatest(false);
    }
  }, [
    onTransactionPageNext,
    onTransactionPagePrev,
    setFollowLatest,
    setTransactionPage,
    transactionPageCount,
  ]);

  return {
    closeDialog,
    openTransactionByHash,
    onTickerListClick,
    onOpportunityListClick,
    onMastheadNavClick,
    onShowRadar,
    onShowOpps,
    onSearchChange,
    onTickerFilterToggle,
    onLiveMainnetFilterChange,
    onTickerFollowClick,
    onTickerToolbarClick,
    onTransactionPagePrev,
    onTransactionPageNext,
    onTransactionPaginationClick,
  };
}
