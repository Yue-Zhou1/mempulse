import { useCallback } from 'react';
import { fetchJson, opportunityRowKey } from '../lib/dashboard-helpers.js';

const archiveTxFetchLimit = 1500;
const archiveOppFetchLimit = 1500;

export function useArchiveRefresh({
  apiBase,
  archiveRequestSeqRef,
  setArchiveLoading,
  setArchiveError,
  setArchiveTxRows,
  setArchiveOppRows,
  setSelectedArchiveTxHash,
  setSelectedArchiveOppKey,
}) {
  return useCallback(async () => {
    const requestSeq = archiveRequestSeqRef.current + 1;
    archiveRequestSeqRef.current = requestSeq;
    setArchiveLoading(true);
    setArchiveError('');

    try {
      const [txPayload, oppPayload] = await Promise.all([
        fetchJson(apiBase, `/transactions/all?limit=${archiveTxFetchLimit}`),
        fetchJson(apiBase, `/opps/recent?limit=${archiveOppFetchLimit}`),
      ]);
      if (archiveRequestSeqRef.current !== requestSeq) {
        return;
      }

      const nextTxRows = Array.isArray(txPayload) ? [...txPayload] : [];
      const nextOppRows = Array.isArray(oppPayload) ? [...oppPayload] : [];

      nextTxRows.sort((left, right) =>
        Number(right?.first_seen_unix_ms ?? 0) - Number(left?.first_seen_unix_ms ?? 0),
      );
      nextOppRows.sort((left, right) =>
        Number(right?.detected_unix_ms ?? 0) - Number(left?.detected_unix_ms ?? 0),
      );

      setArchiveTxRows(nextTxRows);
      setArchiveOppRows(nextOppRows);
      setSelectedArchiveTxHash((current) => {
        if (current && nextTxRows.some((row) => row.hash === current)) {
          return current;
        }
        return nextTxRows[0]?.hash ?? null;
      });
      setSelectedArchiveOppKey((current) => {
        if (current && nextOppRows.some((row) => opportunityRowKey(row) === current)) {
          return current;
        }
        return nextOppRows[0] ? opportunityRowKey(nextOppRows[0]) : null;
      });
    } catch (error) {
      if (archiveRequestSeqRef.current !== requestSeq) {
        return;
      }
      setArchiveError(`Archive sync failed: ${error.message}`);
    } finally {
      if (archiveRequestSeqRef.current === requestSeq) {
        setArchiveLoading(false);
      }
    }
  }, [
    apiBase,
    archiveRequestSeqRef,
    setArchiveError,
    setArchiveLoading,
    setArchiveOppRows,
    setArchiveTxRows,
    setSelectedArchiveOppKey,
    setSelectedArchiveTxHash,
  ]);
}
