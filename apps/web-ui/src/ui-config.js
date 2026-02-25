const DEFAULT_CONFIG = Object.freeze({
  snapshotTxLimit: 120,
  detailCacheLimit: 96,
  maxTransactionHistory: 500,
  maxRenderedTransactions: 80,
  transactionRetentionMs: 5 * 60 * 1000,
});

const LIMITS = Object.freeze({
  snapshotTxLimit: Object.freeze({ min: 1, max: 500 }),
  detailCacheLimit: Object.freeze({ min: 16, max: 1_024 }),
  maxTransactionHistory: Object.freeze({ min: 50, max: 5_000 }),
  maxRenderedTransactions: Object.freeze({ min: 25, max: 1_000 }),
  transactionRetentionMs: Object.freeze({ min: 30_000, max: 3_600_000 }),
});

function parseBoundedInt(env, key, fallback, { min, max }) {
  const raw = env?.[key];
  if (raw == null) {
    return fallback;
  }

  const normalized = String(raw).trim();
  if (!normalized) {
    return fallback;
  }

  const parsed = Number(normalized);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  const integer = Math.floor(parsed);
  return Math.min(max, Math.max(min, integer));
}

function parseBoundedIntFromRaw(raw, { min, max }) {
  if (raw == null) {
    return null;
  }
  const normalized = String(raw).trim();
  if (!normalized) {
    return null;
  }
  const parsed = Number(normalized);
  if (!Number.isFinite(parsed)) {
    return null;
  }
  const integer = Math.floor(parsed);
  return Math.min(max, Math.max(min, integer));
}

function resolveBoundedInt(runtime, runtimeKey, env, envKey, fallback, limits) {
  const runtimeValue = parseBoundedIntFromRaw(runtime?.[runtimeKey], limits);
  if (runtimeValue != null) {
    return runtimeValue;
  }
  return parseBoundedInt(env, envKey, fallback, limits);
}

function resolveEnvInput(env) {
  if (env) {
    return env;
  }
  if (typeof import.meta !== 'undefined' && import.meta && import.meta.env) {
    return import.meta.env;
  }
  return {};
}

function resolveRuntimeInput(runtime) {
  if (runtime && typeof runtime === 'object') {
    return runtime;
  }
  return {};
}

export function readWindowRuntimeConfig() {
  const candidate = globalThis?.window?.__MEMPULSE_UI_CONFIG__;
  if (candidate && typeof candidate === 'object') {
    return candidate;
  }
  return {};
}

export function resolveUiRuntimeConfig({ runtime, env } = {}) {
  const sourceRuntime = resolveRuntimeInput(runtime);
  const sourceEnv = resolveEnvInput(env);

  const snapshotTxLimit = resolveBoundedInt(
    sourceRuntime,
    'txSnapshotLimit',
    sourceEnv,
    'VITE_UI_TX_SNAPSHOT_LIMIT',
    DEFAULT_CONFIG.snapshotTxLimit,
    LIMITS.snapshotTxLimit,
  );
  const detailCacheLimit = resolveBoundedInt(
    sourceRuntime,
    'detailCacheLimit',
    sourceEnv,
    'VITE_UI_DETAIL_CACHE_LIMIT',
    DEFAULT_CONFIG.detailCacheLimit,
    LIMITS.detailCacheLimit,
  );
  const maxTransactionHistory = resolveBoundedInt(
    sourceRuntime,
    'txHistoryLimit',
    sourceEnv,
    'VITE_UI_TX_HISTORY_LIMIT',
    DEFAULT_CONFIG.maxTransactionHistory,
    LIMITS.maxTransactionHistory,
  );
  const configuredRenderLimit = resolveBoundedInt(
    sourceRuntime,
    'txRenderLimit',
    sourceEnv,
    'VITE_UI_TX_RENDER_LIMIT',
    DEFAULT_CONFIG.maxRenderedTransactions,
    LIMITS.maxRenderedTransactions,
  );
  const maxRenderedTransactions = Math.min(configuredRenderLimit, maxTransactionHistory);
  const transactionRetentionMs = resolveBoundedInt(
    sourceRuntime,
    'txRetentionMs',
    sourceEnv,
    'VITE_UI_TX_RETENTION_MS',
    DEFAULT_CONFIG.transactionRetentionMs,
    LIMITS.transactionRetentionMs,
  );
  const transactionRetentionMinutes = Math.max(
    1,
    Math.round(transactionRetentionMs / 60_000),
  );

  return {
    snapshotTxLimit,
    detailCacheLimit,
    maxTransactionHistory,
    maxRenderedTransactions,
    transactionRetentionMs,
    transactionRetentionMinutes,
  };
}
