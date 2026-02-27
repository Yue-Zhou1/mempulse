import test from 'node:test';
import assert from 'node:assert/strict';
import { buildVirtualizedOpportunityWindow } from './opportunity-virtualized-model.js';

test('buildVirtualizedOpportunityWindow bounds rendered row count for large lists', () => {
  const rows = Array.from({ length: 3000 }, (_, index) => ({
    tx_hash: `0x${index.toString(16).padStart(8, '0')}`,
    strategy: 'SandwichCandidate',
    detected_unix_ms: 1_700_000_000_000 + index,
  }));

  const windowModel = buildVirtualizedOpportunityWindow(rows, {
    scrollTop: 0,
    viewportHeight: 480,
    rowHeightPx: 96,
    overscanRows: 4,
  });

  assert.equal(windowModel.totalRowCount, 3000);
  assert.equal(windowModel.startIndex, 0);
  assert.ok(windowModel.endIndex < 25);
  assert.ok(windowModel.visibleRows.length < 25);
  assert.ok(windowModel.paddingBottom > 0);
});

test('buildVirtualizedOpportunityWindow tracks middle-of-list scroll offsets', () => {
  const rows = Array.from({ length: 1000 }, (_, index) => ({
    tx_hash: `0x${index.toString(16).padStart(8, '0')}`,
    strategy: 'BackrunCandidate',
    detected_unix_ms: 1_700_000_000_000 + index,
  }));

  const windowModel = buildVirtualizedOpportunityWindow(rows, {
    scrollTop: 9600,
    viewportHeight: 400,
    rowHeightPx: 100,
    overscanRows: 2,
  });

  assert.ok(windowModel.startIndex >= 94);
  assert.ok(windowModel.startIndex <= 96);
  assert.ok(windowModel.endIndex > windowModel.startIndex);
  assert.equal(windowModel.paddingTop, windowModel.startIndex * 100);
});

test('buildVirtualizedOpportunityWindow normalizes invalid inputs', () => {
  const windowModel = buildVirtualizedOpportunityWindow(null, {
    scrollTop: Number.NaN,
    viewportHeight: 0,
    rowHeightPx: -1,
    overscanRows: -10,
  });

  assert.equal(windowModel.totalRowCount, 0);
  assert.equal(windowModel.totalHeight, 0);
  assert.equal(windowModel.startIndex, 0);
  assert.equal(windowModel.endIndex, 0);
  assert.equal(windowModel.visibleRows.length, 0);
});
