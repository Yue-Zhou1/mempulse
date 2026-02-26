import test from 'node:test';
import assert from 'node:assert/strict';
import {
  classifyRisk,
  formatDurationToken,
  normalizeChainStatusRows,
  opportunityRowKey,
  paginationWindow,
  resolveMainnetLabel,
  sparklinePath,
  statusForRow,
} from './dashboard-helpers.js';

test('resolveMainnetLabel resolves known chains and source fallbacks', () => {
  assert.equal(resolveMainnetLabel(1, 'eth-mainnet'), 'Ethereum');
  assert.equal(resolveMainnetLabel('8453', 'base-mainnet'), 'Base');
  assert.equal(resolveMainnetLabel(null, 'optimism-gateway'), 'Optimism');
  assert.equal(resolveMainnetLabel('abc', 'unknown-source'), 'Unknown');
});

test('normalizeChainStatusRows normalizes and sorts rows', () => {
  const rows = normalizeChainStatusRows([
    {
      chain_key: 'polygon-a',
      chain_id: 137,
      source_id: 'polygon',
      state: 'active',
      endpoint_index: 2,
      endpoint_count: 5,
      ws_url: 'wss://example.invalid/ws',
      http_url: 'https://example.invalid/http',
      last_pending_unix_ms: 123,
      silent_for_ms: 456,
      updated_unix_ms: 789,
      last_error: null,
      rotation_count: 3,
    },
    {
      chain_key: 'base-a',
      source_id: 'base',
    },
  ]);

  assert.equal(rows.length, 2);
  assert.equal(rows[0].chain_key, 'base-a');
  assert.equal(rows[1].chain_key, 'polygon-a');
  assert.equal(rows[0].chain_id, null);
  assert.equal(rows[0].state, 'unknown');
  assert.equal(rows[0].rotation_count, 0);
});

test('risk and status helpers preserve thresholds', () => {
  assert.deepEqual(classifyRisk({ mev_score: 90 }), {
    label: 'High',
    accent: 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]',
  });
  assert.deepEqual(classifyRisk({ mev_score: 50 }), {
    label: 'Medium',
    accent: 'border-zinc-900 text-zinc-800',
  });
  assert.deepEqual(classifyRisk({ mev_score: 10 }), {
    label: 'Low',
    accent: 'border-zinc-700 text-zinc-700',
  });

  assert.equal(statusForRow(null), 'Pending');
  assert.equal(statusForRow({ mev_score: 90, urgency_score: 10 }), 'Flagged');
  assert.equal(statusForRow({ mev_score: 70, urgency_score: 80 }), 'Processing');
  assert.equal(statusForRow({ mev_score: 70, urgency_score: 40 }), 'Completed');
});

test('paginationWindow builds a centered page window', () => {
  assert.deepEqual(paginationWindow(1, 0), [1]);
  assert.deepEqual(paginationWindow(1, 3), [1, 2, 3]);
  assert.deepEqual(paginationWindow(6, 12), [4, 5, 6, 7, 8]);
  assert.deepEqual(paginationWindow(12, 12), [8, 9, 10, 11, 12]);
});

test('formatDurationToken and sparklinePath behave consistently', () => {
  assert.equal(formatDurationToken(-1), '0s');
  assert.equal(formatDurationToken(10_000), '10s');
  assert.equal(formatDurationToken(61_000), '1m');
  assert.equal(formatDurationToken(3_600_000), '1h');

  assert.equal(sparklinePath([], 100, 40), '');
  assert.equal(sparklinePath([1], 100, 40), 'M0.00 0.00');
});

test('opportunityRowKey includes hash strategy and timestamp', () => {
  assert.equal(
    opportunityRowKey({
      tx_hash: '0xabc',
      strategy: 'arb',
      detected_unix_ms: 123456,
    }),
    '0xabc::arb::123456',
  );
  assert.equal(opportunityRowKey({}), '::::');
});
