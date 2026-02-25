import test from 'node:test';
import assert from 'node:assert/strict';
import {
  MAINNET_FILTER_ALL,
  MAINNET_FILTER_OPTIONS,
  normalizeMainnetFilter,
  matchesMainnetFilter,
  filterRowsByMainnet,
} from './mainnet-filter.js';

test('normalizeMainnetFilter falls back to all for invalid values', () => {
  assert.equal(normalizeMainnetFilter(''), MAINNET_FILTER_ALL);
  assert.equal(normalizeMainnetFilter(null), MAINNET_FILTER_ALL);
  assert.equal(normalizeMainnetFilter('not-a-chain'), MAINNET_FILTER_ALL);
});

test('normalizeMainnetFilter accepts known labels case-insensitively', () => {
  assert.equal(normalizeMainnetFilter('ethereum'), 'Ethereum');
  assert.equal(normalizeMainnetFilter('BASE'), 'Base');
});

test('matchesMainnetFilter returns true for all filter', () => {
  assert.equal(matchesMainnetFilter('Ethereum', MAINNET_FILTER_ALL), true);
  assert.equal(matchesMainnetFilter('Base', MAINNET_FILTER_ALL), true);
});

test('matchesMainnetFilter requires exact mainnet label for specific filter', () => {
  assert.equal(matchesMainnetFilter('Ethereum', 'Ethereum'), true);
  assert.equal(matchesMainnetFilter('ethereum', 'Ethereum'), true);
  assert.equal(matchesMainnetFilter('Base', 'Ethereum'), false);
});

test('filterRowsByMainnet keeps rows for selected mainnet only', () => {
  const rows = [
    { id: 1, chain: 'Ethereum' },
    { id: 2, chain: 'Base' },
    { id: 3, chain: 'Optimism' },
  ];

  const filtered = filterRowsByMainnet(rows, (row) => row.chain, 'Base');

  assert.deepEqual(filtered.map((row) => row.id), [2]);
});

test('MAINNET_FILTER_OPTIONS starts with all', () => {
  assert.equal(MAINNET_FILTER_OPTIONS[0]?.value, MAINNET_FILTER_ALL);
});
