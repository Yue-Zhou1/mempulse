import { useCallback } from 'react';
import { fetchJson, opportunityRowKey } from '../lib/dashboard-helpers.js';

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

function archiveTxEquivalent(left, right) {
  return (
    left.hash === right.hash
    && left.first_seen_unix_ms === right.first_seen_unix_ms
    && left.sender === right.sender
    && left.peer === right.peer
    && left.lifecycle_status === right.lifecycle_status
    && left.protocol === right.protocol
    && left.category === right.category
    && left.chain_id === right.chain_id
    && left.source_id === right.source_id
    && left.mev_score === right.mev_score
    && left.urgency_score === right.urgency_score
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

function archiveOppEquivalent(left, right) {
  return (
    left.tx_hash === right.tx_hash
    && left.strategy === right.strategy
    && left.score === right.score
    && left.protocol === right.protocol
    && left.category === right.category
    && left.status === right.status
    && left.chain_id === right.chain_id
    && left.source_id === right.source_id
    && left.detected_unix_ms === right.detected_unix_ms
    && reasonsEquivalent(left.reasons, right.reasons)
  );
}

export function useArchiveRefresh({
  apiBase,
  archiveRequestSeqRef,
  setArchiveLoading,
  setArchiveError,
  setArchiveTxRows,
  setArchiveOppRows,
  setSelectedArchiveTxHash,
  setSelectedArchiveOppKey,
  archiveTxFetchLimit = 600,
  archiveOppFetchLimit = 300,
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

      setArchiveTxRows((current) =>
        stabilizeRows(current, nextTxRows, (row) => row.hash, archiveTxEquivalent),
      );
      setArchiveOppRows((current) =>
        stabilizeRows(current, nextOppRows, opportunityRowKey, archiveOppEquivalent),
      );
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
    archiveTxFetchLimit,
    archiveOppFetchLimit,
    archiveRequestSeqRef,
    setArchiveError,
    setArchiveLoading,
    setArchiveOppRows,
    setArchiveTxRows,
    setSelectedArchiveOppKey,
    setSelectedArchiveTxHash,
  ]);
}
