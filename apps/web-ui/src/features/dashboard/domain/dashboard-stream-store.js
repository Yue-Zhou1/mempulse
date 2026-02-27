import { create } from 'zustand';
import { subscribeWithSelector } from 'zustand/middleware';

const DEFAULT_DETAIL_CACHE_LIMIT = 96;

const transactionDetailCache = new Map();
let pendingTickerCommit = null;
let rafHandle = null;
let rafCancel = null;

function normalizeRows(rows) {
  return Array.isArray(rows) ? rows : [];
}

function resolveNextSelectedHash(currentSelectedHash, transactionRows) {
  if (
    currentSelectedHash
    && transactionRows.some((row) => row?.hash === currentSelectedHash)
  ) {
    return currentSelectedHash;
  }
  return transactionRows[0]?.hash ?? null;
}

function commitTickerRows(set, get, commit) {
  if (!commit) {
    return;
  }
  const transactionRows = normalizeRows(commit.transactionRows);
  const recentTxRows = normalizeRows(commit.recentTxRows);

  set((current) => {
    const nextSelectedHash = resolveNextSelectedHash(
      current.activeSelectedTxId,
      transactionRows,
    );
    if (
      current.transactionRows === transactionRows
      && current.recentTxRows === recentTxRows
      && current.activeSelectedTxId === nextSelectedHash
    ) {
      return current;
    }
    return {
      ...current,
      transactionRows,
      recentTxRows,
      activeSelectedTxId: nextSelectedHash,
    };
  });

  pendingTickerCommit = null;
  if (rafHandle != null) {
    rafHandle = null;
    rafCancel = null;
  }

  const liveHashes = new Set(transactionRows.map((row) => row.hash));
  if (pruneTransactionDetailCache(liveHashes)) {
    get().bumpDetailVersion();
  }
}

function resolveRequestFrame(options) {
  if (typeof options?.requestFrame === 'function') {
    return options.requestFrame;
  }
  if (typeof globalThis?.requestAnimationFrame === 'function') {
    return globalThis.requestAnimationFrame.bind(globalThis);
  }
  if (typeof globalThis?.setTimeout === 'function') {
    return (callback) => globalThis.setTimeout(() => callback(Date.now()), 0);
  }
  return null;
}

function resolveCancelFrame(options) {
  if (typeof options?.cancelFrame === 'function') {
    return options.cancelFrame;
  }
  if (typeof globalThis?.cancelAnimationFrame === 'function') {
    return globalThis.cancelAnimationFrame.bind(globalThis);
  }
  if (typeof globalThis?.clearTimeout === 'function') {
    return globalThis.clearTimeout.bind(globalThis);
  }
  return null;
}

export const useDashboardStreamStore = create(
  subscribeWithSelector((set, get) => ({
    recentTxRows: [],
    transactionRows: [],
    activeSelectedTxId: null,
    detailVersion: 0,
    commitTickerRows: (commit) => {
      commitTickerRows(set, get, commit);
    },
    enqueueTickerCommit: (commit, options = {}) => {
      pendingTickerCommit = commit;
      if (rafHandle != null) {
        return;
      }

      const requestFrame = resolveRequestFrame(options);
      rafCancel = resolveCancelFrame(options);
      if (!requestFrame) {
        commitTickerRows(set, get, pendingTickerCommit);
        return;
      }

      rafHandle = requestFrame((timestamp) => {
        void timestamp;
        rafHandle = null;
        rafCancel = null;
        commitTickerRows(set, get, pendingTickerCommit);
      });
    },
    flushTickerCommit: () => {
      if (!pendingTickerCommit) {
        return;
      }
      if (rafHandle != null && typeof rafCancel === 'function') {
        rafCancel(rafHandle);
      }
      rafHandle = null;
      rafCancel = null;
      commitTickerRows(set, get, pendingTickerCommit);
    },
    cancelTickerCommit: () => {
      pendingTickerCommit = null;
      if (rafHandle != null && typeof rafCancel === 'function') {
        rafCancel(rafHandle);
      }
      rafHandle = null;
      rafCancel = null;
    },
    setActiveSelectedTxId: (hash) => {
      set((current) => {
        if (current.activeSelectedTxId === hash) {
          return current;
        }
        return {
          ...current,
          activeSelectedTxId: hash,
        };
      });
    },
    bumpDetailVersion: () => {
      set((current) => ({
        ...current,
        detailVersion: current.detailVersion + 1,
      }));
    },
    reset: () => {
      pendingTickerCommit = null;
      if (rafHandle != null && typeof rafCancel === 'function') {
        rafCancel(rafHandle);
      }
      rafHandle = null;
      rafCancel = null;
      set({
        recentTxRows: [],
        transactionRows: [],
        activeSelectedTxId: null,
        detailVersion: 0,
      });
    },
  })),
);

export function getTransactionDetailByHash(hash) {
  if (!hash) {
    return null;
  }
  return transactionDetailCache.get(hash) ?? null;
}

export function setTransactionDetailByHash(hash, detail, limit = DEFAULT_DETAIL_CACHE_LIMIT) {
  if (!hash) {
    return false;
  }
  const hasPrevious = transactionDetailCache.has(hash);
  if (hasPrevious) {
    transactionDetailCache.delete(hash);
  }
  transactionDetailCache.set(hash, detail);

  const boundedLimit = Number.isFinite(limit)
    ? Math.max(1, Math.floor(limit))
    : DEFAULT_DETAIL_CACHE_LIMIT;
  while (transactionDetailCache.size > boundedLimit) {
    const oldestKey = transactionDetailCache.keys().next().value;
    transactionDetailCache.delete(oldestKey);
  }

  useDashboardStreamStore.getState().bumpDetailVersion();
  return true;
}

export function pruneTransactionDetailCache(liveHashes) {
  let changed = false;
  for (const hash of transactionDetailCache.keys()) {
    if (!liveHashes.has(hash)) {
      transactionDetailCache.delete(hash);
      changed = true;
    }
  }
  return changed;
}

export function clearTransactionDetailCache() {
  if (transactionDetailCache.size === 0) {
    return false;
  }
  transactionDetailCache.clear();
  useDashboardStreamStore.getState().bumpDetailVersion();
  return true;
}

export function __resetDashboardStreamStoreForTests() {
  pendingTickerCommit = null;
  if (rafHandle != null && typeof rafCancel === 'function') {
    rafCancel(rafHandle);
  }
  rafHandle = null;
  rafCancel = null;
  transactionDetailCache.clear();
  useDashboardStreamStore.getState().reset();
}
