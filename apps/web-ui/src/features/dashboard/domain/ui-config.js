const DEFAULT_CONFIG = Object.freeze({
  snapshotTxLimit: 120,
  detailCacheLimit: 96,
  maxTransactionHistory: 500,
  maxFeatureHistory: 500,
  maxOpportunityHistory: 500,
  maxRenderedTransactions: 500,
  streamBatchMs: 1_000,
  samplingLagThresholdMs: 30,
  samplingStride: 5,
  samplingFlushIdleMs: 500,
  heapEmergencyPurgeMb: 400,
  devPerformanceEntryCleanupIntervalMs: 15_000,
  transactionRetentionMs: 5 * 60 * 1000,
  streamTransport: 'sse',
  workerEnabled: true,
  virtualizedTickerEnabled: true,
  devPerformanceEntryCleanupEnabled: true,
});

const LIMITS = Object.freeze({
  snapshotTxLimit: Object.freeze({ min: 1, max: 500 }),
  detailCacheLimit: Object.freeze({ min: 16, max: 1_024 }),
  maxTransactionHistory: Object.freeze({ min: 50, max: 5_000 }),
  maxFeatureHistory: Object.freeze({ min: 50, max: 10_000 }),
  maxOpportunityHistory: Object.freeze({ min: 50, max: 10_000 }),
  maxRenderedTransactions: Object.freeze({ min: 25, max: 1_000 }),
  streamBatchMs: Object.freeze({ min: 100, max: 2_000 }),
  samplingLagThresholdMs: Object.freeze({ min: 16, max: 120 }),
  samplingStride: Object.freeze({ min: 1, max: 20 }),
  samplingFlushIdleMs: Object.freeze({ min: 100, max: 2_000 }),
  heapEmergencyPurgeMb: Object.freeze({ min: 128, max: 1_024 }),
  devPerformanceEntryCleanupIntervalMs: Object.freeze({ min: 5_000, max: 120_000 }),
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
    configKey: 'maxFeatureHistory',
    runtimeKey: 'featureHistoryLimit',
    envKey: 'VITE_UI_FEATURE_HISTORY_LIMIT',
    limitsKey: 'maxFeatureHistory',
  }),
  Object.freeze({
    configKey: 'maxOpportunityHistory',
    runtimeKey: 'opportunityHistoryLimit',
    envKey: 'VITE_UI_OPPORTUNITY_HISTORY_LIMIT',
    limitsKey: 'maxOpportunityHistory',
  }),
  Object.freeze({
    configKey: 'maxRenderedTransactions',
    runtimeKey: 'txRenderLimit',
    envKey: 'VITE_UI_TX_RENDER_LIMIT',
    limitsKey: 'maxRenderedTransactions',
  }),
  Object.freeze({
    configKey: 'streamBatchMs',
    runtimeKey: 'streamBatchMs',
    envKey: 'VITE_UI_STREAM_BATCH_MS',
    limitsKey: 'streamBatchMs',
  }),
  Object.freeze({
    configKey: 'samplingLagThresholdMs',
    runtimeKey: 'samplingLagThresholdMs',
    envKey: 'VITE_UI_SAMPLING_LAG_THRESHOLD_MS',
    limitsKey: 'samplingLagThresholdMs',
  }),
  Object.freeze({
    configKey: 'samplingStride',
    runtimeKey: 'samplingStride',
    envKey: 'VITE_UI_SAMPLING_STRIDE',
    limitsKey: 'samplingStride',
  }),
  Object.freeze({
    configKey: 'samplingFlushIdleMs',
    runtimeKey: 'samplingFlushIdleMs',
    envKey: 'VITE_UI_SAMPLING_FLUSH_IDLE_MS',
    limitsKey: 'samplingFlushIdleMs',
  }),
  Object.freeze({
    configKey: 'heapEmergencyPurgeMb',
    runtimeKey: 'heapEmergencyPurgeMb',
    envKey: 'VITE_UI_HEAP_EMERGENCY_PURGE_MB',
    limitsKey: 'heapEmergencyPurgeMb',
  }),
  Object.freeze({
    configKey: 'devPerformanceEntryCleanupIntervalMs',
    runtimeKey: 'devPerformanceEntryCleanupIntervalMs',
    envKey: 'VITE_UI_DEV_PERF_ENTRY_CLEANUP_INTERVAL_MS',
    limitsKey: 'devPerformanceEntryCleanupIntervalMs',
  }),
  Object.freeze({
    configKey: 'transactionRetentionMs',
    runtimeKey: 'txRetentionMs',
    envKey: 'VITE_UI_TX_RETENTION_MS',
    limitsKey: 'transactionRetentionMs',
  }),
]);

const BOOLEAN_CONFIG_RESOLVERS = Object.freeze([
  Object.freeze({
    configKey: 'workerEnabled',
    runtimeKey: 'workerEnabled',
    envKey: 'VITE_UI_WORKER_ENABLED',
  }),
  Object.freeze({
    configKey: 'virtualizedTickerEnabled',
    runtimeKey: 'virtualizedTickerEnabled',
    envKey: 'VITE_UI_VIRTUALIZED_TICKER_ENABLED',
  }),
  Object.freeze({
    configKey: 'devPerformanceEntryCleanupEnabled',
    runtimeKey: 'devPerformanceEntryCleanupEnabled',
    envKey: 'VITE_UI_DEV_PERF_ENTRY_CLEANUP',
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

function parseBooleanValue(raw) {
  if (raw == null) {
    return null;
  }
  if (typeof raw === 'boolean') {
    return raw;
  }
  const normalized = String(raw).trim().toLowerCase();
  if (!normalized) {
    return null;
  }
  if (['1', 'true', 'yes', 'on'].includes(normalized)) {
    return true;
  }
  if (['0', 'false', 'no', 'off'].includes(normalized)) {
    return false;
  }
  return null;
}

function resolveBoolean(runtime, runtimeKey, env, envKey, fallback) {
  const runtimeValue = parseBooleanValue(runtime?.[runtimeKey]);
  if (runtimeValue != null) {
    return runtimeValue;
  }
  const envValue = parseBooleanValue(env?.[envKey]);
  if (envValue != null) {
    return envValue;
  }
  return fallback;
}

function parseStreamTransportValue(raw) {
  if (raw == null) {
    return null;
  }
  const normalized = String(raw).trim().toLowerCase();
  if (normalized === 'sse' || normalized === 'ws') {
    return normalized;
  }
  return null;
}

function resolveStreamTransport(runtime, env, fallback) {
  const runtimeValue = parseStreamTransportValue(runtime?.streamTransport);
  if (runtimeValue != null) {
    return runtimeValue;
  }
  const envValue = parseStreamTransportValue(env?.VITE_UI_STREAM_TRANSPORT);
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
  for (const resolver of BOOLEAN_CONFIG_RESOLVERS) {
    resolved[resolver.configKey] = resolveBoolean(
      sourceRuntime,
      resolver.runtimeKey,
      sourceEnv,
      resolver.envKey,
      DEFAULT_CONFIG[resolver.configKey],
    );
  }
  resolved.streamTransport = resolveStreamTransport(
    sourceRuntime,
    sourceEnv,
    DEFAULT_CONFIG.streamTransport,
  );

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
    maxFeatureHistory: resolved.maxFeatureHistory,
    maxOpportunityHistory: resolved.maxOpportunityHistory,
    maxRenderedTransactions,
    streamBatchMs: resolved.streamBatchMs,
    samplingLagThresholdMs: resolved.samplingLagThresholdMs,
    samplingStride: resolved.samplingStride,
    samplingFlushIdleMs: resolved.samplingFlushIdleMs,
    heapEmergencyPurgeMb: resolved.heapEmergencyPurgeMb,
    devPerformanceEntryCleanupIntervalMs: resolved.devPerformanceEntryCleanupIntervalMs,
    transactionRetentionMs: resolved.transactionRetentionMs,
    transactionRetentionMinutes,
    streamTransport: resolved.streamTransport,
    workerEnabled: resolved.workerEnabled,
    virtualizedTickerEnabled: resolved.virtualizedTickerEnabled,
    devPerformanceEntryCleanupEnabled: resolved.devPerformanceEntryCleanupEnabled,
  };
}
