import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveUiRuntimeConfig } from './ui-config.js';

test('resolveUiRuntimeConfig returns defaults when env is empty', () => {
  const config = resolveUiRuntimeConfig({ env: {} });

  assert.equal(config.snapshotTxLimit, 120);
  assert.equal(config.detailCacheLimit, 96);
  assert.equal(config.maxTransactionHistory, 500);
  assert.equal(config.maxRenderedTransactions, 80);
  assert.equal(config.transactionRetentionMs, 300_000);
  assert.equal(config.transactionRetentionMinutes, 5);
});

test('resolveUiRuntimeConfig accepts valid env overrides', () => {
  const config = resolveUiRuntimeConfig({
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '200',
      VITE_UI_DETAIL_CACHE_LIMIT: '140',
      VITE_UI_TX_HISTORY_LIMIT: '900',
      VITE_UI_TX_RENDER_LIMIT: '220',
      VITE_UI_TX_RETENTION_MS: '900000',
    },
  });

  assert.equal(config.snapshotTxLimit, 200);
  assert.equal(config.detailCacheLimit, 140);
  assert.equal(config.maxTransactionHistory, 900);
  assert.equal(config.maxRenderedTransactions, 220);
  assert.equal(config.transactionRetentionMs, 900_000);
  assert.equal(config.transactionRetentionMinutes, 15);
});

test('resolveUiRuntimeConfig runtime config overrides env values', () => {
  const config = resolveUiRuntimeConfig({
    runtime: {
      txSnapshotLimit: 180,
      detailCacheLimit: 110,
      txHistoryLimit: 400,
      txRenderLimit: 90,
      txRetentionMs: 420_000,
    },
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '200',
      VITE_UI_DETAIL_CACHE_LIMIT: '140',
      VITE_UI_TX_HISTORY_LIMIT: '900',
      VITE_UI_TX_RENDER_LIMIT: '220',
      VITE_UI_TX_RETENTION_MS: '900000',
    },
  });

  assert.equal(config.snapshotTxLimit, 180);
  assert.equal(config.detailCacheLimit, 110);
  assert.equal(config.maxTransactionHistory, 400);
  assert.equal(config.maxRenderedTransactions, 90);
  assert.equal(config.transactionRetentionMs, 420_000);
  assert.equal(config.transactionRetentionMinutes, 7);
});

test('resolveUiRuntimeConfig clamps invalid values and render does not exceed history', () => {
  const config = resolveUiRuntimeConfig({
    env: {
      VITE_UI_TX_SNAPSHOT_LIMIT: '9999',
      VITE_UI_DETAIL_CACHE_LIMIT: '2',
      VITE_UI_TX_HISTORY_LIMIT: '80',
      VITE_UI_TX_RENDER_LIMIT: '1000',
      VITE_UI_TX_RETENTION_MS: '1000',
    },
  });

  assert.equal(config.snapshotTxLimit, 500);
  assert.equal(config.detailCacheLimit, 16);
  assert.equal(config.maxTransactionHistory, 80);
  assert.equal(config.maxRenderedTransactions, 80);
  assert.equal(config.transactionRetentionMs, 30_000);
  assert.equal(config.transactionRetentionMinutes, 1);
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
  assert.equal(config.maxRenderedTransactions, 80);
});
