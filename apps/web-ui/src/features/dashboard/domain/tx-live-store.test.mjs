import test from 'node:test';
import assert from 'node:assert/strict';
import { createTxLiveStore } from './tx-live-store.js';

test('tx live store keeps bounded rows and deduplicates by hash with incoming priority', () => {
  const store = createTxLiveStore();
  const first = store.snapshot(
    [
      { hash: '0x03', sender: '0xc', seen_unix_ms: 1030 },
      { hash: '0x02', sender: '0xb2', seen_unix_ms: 1020 },
      { hash: '0x01', sender: '0xa', seen_unix_ms: 1010 },
    ],
    { maxItems: 3, nowUnixMs: 2000, maxAgeMs: 10_000 },
  );
  assert.equal(first.rows.length, 3);
  assert.deepEqual(first.rows.map((row) => row.hash), ['0x03', '0x02', '0x01']);

  const second = store.snapshot(
    [
      { hash: '0x04', sender: '0xd', seen_unix_ms: 1040 },
      { hash: '0x02', sender: '0xb3', seen_unix_ms: 1025 },
    ],
    { maxItems: 3, nowUnixMs: 2000, maxAgeMs: 10_000 },
  );
  assert.equal(second.rows.length, 3);
  assert.deepEqual(second.rows.map((row) => row.hash), ['0x04', '0x02', '0x03']);
  assert.equal(second.rows[1].sender, '0xb3');
});

test('tx live store preserves row identity when incoming row is unchanged', () => {
  const store = createTxLiveStore();
  const row = {
    hash: '0xaaa',
    sender: '0x111',
    nonce: 7,
    tx_type: 2,
    seen_unix_ms: 123,
    source_id: 'eth-mainnet',
    chain_id: 1,
  };
  store.snapshot([row], { maxItems: 10, nowUnixMs: 1000, maxAgeMs: 10_000 });

  const next = store.snapshot(
    [
      {
        hash: '0xaaa',
        sender: '0x111',
        nonce: 7,
        tx_type: 2,
        seen_unix_ms: 123,
        source_id: 'eth-mainnet',
        chain_id: 1,
      },
    ],
    { maxItems: 10, nowUnixMs: 1000, maxAgeMs: 10_000 },
  );

  assert.equal(next.rows.length, 1);
  assert.equal(next.rows[0], row);
});

test('tx live store returns the same rows array reference when no effective change', () => {
  const store = createTxLiveStore();
  const first = store.snapshot(
    [
      { hash: '0x01', sender: '0xa', seen_unix_ms: 1001 },
      { hash: '0x02', sender: '0xb', seen_unix_ms: 1000 },
    ],
    { maxItems: 10, nowUnixMs: 2000, maxAgeMs: 10_000 },
  );
  const second = store.snapshot(
    [
      { hash: '0x01', sender: '0xa', seen_unix_ms: 1001 },
      { hash: '0x02', sender: '0xb', seen_unix_ms: 1000 },
    ],
    { maxItems: 10, nowUnixMs: 2000, maxAgeMs: 10_000 },
  );

  assert.equal(second.rows, first.rows);
  assert.equal(second.changed, false);
});

test('tx live store prunes rows older than max age window', () => {
  const store = createTxLiveStore();
  store.snapshot(
    [
      { hash: '0x01', seen_unix_ms: 1000 },
      { hash: '0x02', seen_unix_ms: 1500 },
    ],
    { maxItems: 10, nowUnixMs: 2000, maxAgeMs: 10_000 },
  );

  const next = store.snapshot(
    [{ hash: '0x03', seen_unix_ms: 1990 }],
    { maxItems: 10, nowUnixMs: 2600, maxAgeMs: 800 },
  );
  assert.deepEqual(next.rows.map((row) => row.hash), ['0x03']);
});
