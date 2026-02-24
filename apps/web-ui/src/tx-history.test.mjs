import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeTransactionHistory } from './tx-history.js';

test('mergeTransactionHistory prefers newest batch and deduplicates by hash', () => {
  const existing = [
    { hash: '0x01', sender: '0xa' },
    { hash: '0x02', sender: '0xb' },
  ];
  const incoming = [
    { hash: '0x03', sender: '0xc' },
    { hash: '0x02', sender: '0xbb' },
  ];

  const merged = mergeTransactionHistory(existing, incoming, 10);

  assert.deepEqual(
    merged.map((row) => row.hash),
    ['0x03', '0x02', '0x01'],
  );
  assert.equal(merged[1].sender, '0xbb');
});

test('mergeTransactionHistory enforces max item cap', () => {
  const existing = [
    { hash: '0x05' },
    { hash: '0x04' },
    { hash: '0x03' },
  ];
  const incoming = [{ hash: '0x02' }, { hash: '0x01' }];

  const merged = mergeTransactionHistory(existing, incoming, 4);

  assert.equal(merged.length, 4);
  assert.deepEqual(
    merged.map((row) => row.hash),
    ['0x02', '0x01', '0x05', '0x04'],
  );
});

test('mergeTransactionHistory returns empty rows when max is zero', () => {
  const merged = mergeTransactionHistory([{ hash: '0x01' }], [{ hash: '0x02' }], 0);
  assert.deepEqual(merged, []);
});

test('mergeTransactionHistory prunes rows older than the live window', () => {
  const nowUnixMs = 1_000_000;
  const merged = mergeTransactionHistory(
    [
      { hash: '0x01', seen_unix_ms: 989_000 },
      { hash: '0x02', seen_unix_ms: 995_500 },
    ],
    [
      { hash: '0x03', seen_unix_ms: 999_000 },
      { hash: '0x02', seen_unix_ms: 995_500 },
    ],
    10,
    { nowUnixMs, maxAgeMs: 10_000 },
  );

  assert.deepEqual(
    merged.map((row) => row.hash),
    ['0x03', '0x02'],
  );
});
