const DEFAULT_CONFIG = Object.freeze({
  snapshotTxLimit: 120,
  detailCacheLimit: 96,
  maxTransactionHistory: 500,
  maxRenderedTransactions: 500,
  transactionRetentionMs: 5 * 60 * 1000,
});

const LIMITS = Object.freeze({
  snapshotTxLimit: Object.freeze({ min: 1, max: 500 }),
  detailCacheLimit: Object.freeze({ min: 16, max: 1_024 }),
  maxTransactionHistory: Object.freeze({ min: 50, max: 5_000 }),
  maxRenderedTransactions: Object.freeze({ min: 25, max: 1_000 }),
  transactionRetentionMs: Object.freeze({ min: 30_000, max: 3_600_000 }),
});

const CONFIG_RESOLVERS = Object.freeze([
  Object.freeze({
    configKey: 'snapshotTxLimit',
    runtimeKey: 'txSnapshotLimit',
    envKey: 'VITE_UI_TX_SNAPSHOT_LIMIT',
    limitsKey: 'snapshotTxLimit',
  }),
  Object.freeze({
    configKey: 'detailCacheLimit',
    runtimeKey: 'detailCacheLimit',
    envKey: 'VITE_UI_DETAIL_CACHE_LIMIT',
    limitsKey: 'detailCacheLimit',
  }),
  Object.freeze({
    configKey: 'maxTransactionHistory',
    runtimeKey: 'txHistoryLimit',
    envKey: 'VITE_UI_TX_HISTORY_LIMIT',
    limitsKey: 'maxTransactionHistory',
  }),
  Object.freeze({
    configKey: 'maxRenderedTransactions',
    runtimeKey: 'txRenderLimit',
    envKey: 'VITE_UI_TX_RENDER_LIMIT',
    limitsKey: 'maxRenderedTransactions',
  }),
  Object.freeze({
    configKey: 'transactionRetentionMs',
    runtimeKey: 'txRetentionMs',
    envKey: 'VITE_UI_TX_RETENTION_MS',
    limitsKey: 'transactionRetentionMs',
  }),
]);

function parseBoundedIntValue(raw, { min, max }) {
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
  const runtimeValue = parseBoundedIntValue(runtime?.[runtimeKey], limits);
  if (runtimeValue != null) {
    return runtimeValue;
  }

  const envValue = parseBoundedIntValue(env?.[envKey], limits);
  if (envValue != null) {
    return envValue;
  }

  return fallback;
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

  const resolved = {};
  for (const resolver of CONFIG_RESOLVERS) {
    resolved[resolver.configKey] = resolveBoundedInt(
      sourceRuntime,
      resolver.runtimeKey,
      sourceEnv,
      resolver.envKey,
      DEFAULT_CONFIG[resolver.configKey],
      LIMITS[resolver.limitsKey],
    );
  }

  const maxRenderedTransactions = Math.min(
    resolved.maxRenderedTransactions,
    resolved.maxTransactionHistory,
  );
  const transactionRetentionMinutes = Math.max(
    1,
    Math.round(resolved.transactionRetentionMs / 60_000),
  );

  return {
    snapshotTxLimit: resolved.snapshotTxLimit,
    detailCacheLimit: resolved.detailCacheLimit,
    maxTransactionHistory: resolved.maxTransactionHistory,
    maxRenderedTransactions,
    transactionRetentionMs: resolved.transactionRetentionMs,
    transactionRetentionMinutes,
  };
}
