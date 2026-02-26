import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveUiRuntimeConfig } from './ui-config.js';

test('resolveUiRuntimeConfig returns defaults when env is empty', () => {
  const config = resolveUiRuntimeConfig({ env: {} });

  assert.equal(config.snapshotTxLimit, 120);
  assert.equal(config.detailCacheLimit, 96);
  assert.equal(config.maxTransactionHistory, 500);
  assert.equal(config.maxFeatureHistory, 1500);
  assert.equal(config.maxOpportunityHistory, 1500);
  assert.equal(config.maxRenderedTransactions, 500);
  assert.equal(config.transactionRetentionMs, 300_000);
  assert.equal(config.transactionRetentionMinutes, 5);
  assert.equal(config.workerEnabled, true);
  assert.equal(config.virtualizedTickerEnabled, true);
  assert.equal(config.streamBatchMs, 1000);
  assert.equal(config.samplingLagThresholdMs, 30);
  assert.equal(config.samplingStride, 5);
  assert.equal(config.samplingFlushIdleMs, 500);
  assert.equal(config.heapEmergencyPurgeMb, 400);
});

test('resolveUiRuntimeConfig accepts valid env overrides', () => {
  const config = resolveUiRuntimeConfig({
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '200',
      VITE_UI_DETAIL_CACHE_LIMIT: '140',
      VITE_UI_TX_HISTORY_LIMIT: '900',
      VITE_UI_FEATURE_HISTORY_LIMIT: '1900',
      VITE_UI_OPPORTUNITY_HISTORY_LIMIT: '2100',
      VITE_UI_TX_RENDER_LIMIT: '220',
      VITE_UI_TX_RETENTION_MS: '900000',
      VITE_UI_WORKER_ENABLED: 'false',
      VITE_UI_VIRTUALIZED_TICKER_ENABLED: '0',
      VITE_UI_STREAM_BATCH_MS: '145',
      VITE_UI_SAMPLING_LAG_THRESHOLD_MS: '42',
      VITE_UI_SAMPLING_STRIDE: '7',
      VITE_UI_SAMPLING_FLUSH_IDLE_MS: '650',
      VITE_UI_HEAP_EMERGENCY_PURGE_MB: '512',
    },
  });

  assert.equal(config.snapshotTxLimit, 200);
  assert.equal(config.detailCacheLimit, 140);
  assert.equal(config.maxTransactionHistory, 900);
  assert.equal(config.maxFeatureHistory, 1900);
  assert.equal(config.maxOpportunityHistory, 2100);
  assert.equal(config.maxRenderedTransactions, 220);
  assert.equal(config.transactionRetentionMs, 900_000);
  assert.equal(config.transactionRetentionMinutes, 15);
  assert.equal(config.workerEnabled, false);
  assert.equal(config.virtualizedTickerEnabled, false);
  assert.equal(config.streamBatchMs, 145);
  assert.equal(config.samplingLagThresholdMs, 42);
  assert.equal(config.samplingStride, 7);
  assert.equal(config.samplingFlushIdleMs, 650);
  assert.equal(config.heapEmergencyPurgeMb, 512);
});

test('resolveUiRuntimeConfig runtime config overrides env values', () => {
  const config = resolveUiRuntimeConfig({
    runtime: {
      txSnapshotLimit: 180,
      detailCacheLimit: 110,
      txHistoryLimit: 400,
      featureHistoryLimit: 1300,
      opportunityHistoryLimit: 800,
      txRenderLimit: 90,
      txRetentionMs: 420_000,
      workerEnabled: false,
      virtualizedTickerEnabled: false,
      streamBatchMs: 130,
      samplingLagThresholdMs: 33,
      samplingStride: 6,
      samplingFlushIdleMs: 540,
      heapEmergencyPurgeMb: 450,
    },
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '200',
      VITE_UI_DETAIL_CACHE_LIMIT: '140',
      VITE_UI_TX_HISTORY_LIMIT: '900',
      VITE_UI_FEATURE_HISTORY_LIMIT: '1900',
      VITE_UI_OPPORTUNITY_HISTORY_LIMIT: '2100',
      VITE_UI_TX_RENDER_LIMIT: '220',
      VITE_UI_TX_RETENTION_MS: '900000',
      VITE_UI_WORKER_ENABLED: 'true',
      VITE_UI_VIRTUALIZED_TICKER_ENABLED: 'true',
      VITE_UI_STREAM_BATCH_MS: '145',
      VITE_UI_SAMPLING_LAG_THRESHOLD_MS: '42',
      VITE_UI_SAMPLING_STRIDE: '7',
      VITE_UI_SAMPLING_FLUSH_IDLE_MS: '650',
      VITE_UI_HEAP_EMERGENCY_PURGE_MB: '512',
    },
  });

  assert.equal(config.snapshotTxLimit, 180);
  assert.equal(config.detailCacheLimit, 110);
  assert.equal(config.maxTransactionHistory, 400);
  assert.equal(config.maxFeatureHistory, 1300);
  assert.equal(config.maxOpportunityHistory, 800);
  assert.equal(config.maxRenderedTransactions, 90);
  assert.equal(config.transactionRetentionMs, 420_000);
  assert.equal(config.transactionRetentionMinutes, 7);
  assert.equal(config.workerEnabled, false);
  assert.equal(config.virtualizedTickerEnabled, false);
  assert.equal(config.streamBatchMs, 130);
  assert.equal(config.samplingLagThresholdMs, 33);
  assert.equal(config.samplingStride, 6);
  assert.equal(config.samplingFlushIdleMs, 540);
  assert.equal(config.heapEmergencyPurgeMb, 450);
});

test('resolveUiRuntimeConfig clamps invalid values and render does not exceed history', () => {
  const config = resolveUiRuntimeConfig({
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '9999',
      VITE_UI_DETAIL_CACHE_LIMIT: '2',
      VITE_UI_TX_HISTORY_LIMIT: '80',
      VITE_UI_FEATURE_HISTORY_LIMIT: '999999',
      VITE_UI_OPPORTUNITY_HISTORY_LIMIT: '0',
      VITE_UI_TX_RENDER_LIMIT: '1000',
      VITE_UI_TX_RETENTION_MS: '1000',
      VITE_UI_STREAM_BATCH_MS: '9999',
      VITE_UI_SAMPLING_LAG_THRESHOLD_MS: '-1',
      VITE_UI_SAMPLING_STRIDE: '0',
      VITE_UI_SAMPLING_FLUSH_IDLE_MS: '5',
      VITE_UI_HEAP_EMERGENCY_PURGE_MB: '9999',
    },
  });

  assert.equal(config.snapshotTxLimit, 500);
  assert.equal(config.detailCacheLimit, 16);
  assert.equal(config.maxTransactionHistory, 80);
  assert.equal(config.maxFeatureHistory, 10000);
  assert.equal(config.maxOpportunityHistory, 50);
  assert.equal(config.maxRenderedTransactions, 80);
  assert.equal(config.transactionRetentionMs, 30_000);
  assert.equal(config.transactionRetentionMinutes, 1);
  assert.equal(config.streamBatchMs, 2000);
  assert.equal(config.samplingLagThresholdMs, 16);
  assert.equal(config.samplingStride, 1);
  assert.equal(config.samplingFlushIdleMs, 100);
  assert.equal(config.heapEmergencyPurgeMb, 1024);
});

test('resolveUiRuntimeConfig ignores blank runtime values and falls back to env/defaults', () => {
  const config = resolveUiRuntimeConfig({
    runtime: {
      txSnapshotLimit: '   ',
      txRenderLimit: null,
    },
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '150',
    },
  });

  assert.equal(config.snapshotTxLimit, 150);
  assert.equal(config.maxRenderedTransactions, 500);
});
