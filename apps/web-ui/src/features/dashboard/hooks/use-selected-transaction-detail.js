import { useMemo } from 'react';
import {
  getTransactionDetailByHash,
  useDashboardStreamStore,
} from '../domain/dashboard-stream-store.js';

export function useSelectedTransactionDetail(selectedTxId) {
  const detailVersion = useDashboardStreamStore((state) => state.detailVersion);

  return useMemo(() => {
    if (!selectedTxId) {
      return null;
    }
    return getTransactionDetailByHash(selectedTxId);
  }, [detailVersion, selectedTxId]);
}
