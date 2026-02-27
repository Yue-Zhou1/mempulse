import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildVirtualizedTickerRows,
  rowsFromVirtualizedModels,
  resolveVirtualizedSelectionIndex,
} from './radar-virtualized-model.js';

test('buildVirtualizedTickerRows keeps stable slot keys and row references', () => {
  const rows = [
    { hash: '0xaaa', sender: '0x1' },
    { hash: '0xbbb', sender: '0x2' },
  ];
  const first = buildVirtualizedTickerRows(rows, 5);
  const second = buildVirtualizedTickerRows(rows, 5);

  assert.deepEqual(first.map((row) => row.key), ['slot-0', 'slot-1', 'slot-2', 'slot-3', 'slot-4']);
  assert.deepEqual(second.map((row) => row.key), ['slot-0', 'slot-1', 'slot-2', 'slot-3', 'slot-4']);
  assert.equal(first[0].row, rows[0]);
  assert.equal(first[1].row, rows[1]);
  assert.equal(first[2].row, null);
});

test('buildVirtualizedTickerRows reuses previous models when rows are unchanged', () => {
  const rows = [
    { hash: '0xaaa', sender: '0x1' },
    { hash: '0xbbb', sender: '0x2' },
  ];
  const first = buildVirtualizedTickerRows(rows, 5);
  const rowsWithNewArrayIdentity = rows.slice();
  const second = buildVirtualizedTickerRows(rowsWithNewArrayIdentity, 5, first);

  assert.equal(second, first);
  assert.equal(second[0], first[0]);
  assert.equal(second[1], first[1]);
});

test('buildVirtualizedTickerRows enforces a hard 50-slot table size', () => {
  const rows = Array.from({ length: 80 }, (_, index) => ({
    hash: `0x${index.toString(16).padStart(4, '0')}`,
  }));

  const models = buildVirtualizedTickerRows(rows, 120);

  assert.equal(models.length, 50);
  assert.equal(models[0].key, 'slot-0');
  assert.equal(models[49].key, 'slot-49');
});

test('resolveVirtualizedSelectionIndex accounts for visible window offset', () => {
  const rows = [
    { hash: '0x01' },
    { hash: '0x02' },
    { hash: '0x03' },
  ];

  assert.equal(resolveVirtualizedSelectionIndex(rows, '0x03', 40), 42);
  assert.equal(resolveVirtualizedSelectionIndex(rows, '0xmissing', 40), -1);
});

test('rowsFromVirtualizedModels drops padded null slots', () => {
  const sourceRows = [
    { hash: '0xaaa', sender: '0x1' },
    { hash: '0xbbb', sender: '0x2' },
  ];
  const virtualizedRows = buildVirtualizedTickerRows(sourceRows, 5);

  const rows = rowsFromVirtualizedModels(virtualizedRows);

  assert.deepEqual(rows, sourceRows);
});
