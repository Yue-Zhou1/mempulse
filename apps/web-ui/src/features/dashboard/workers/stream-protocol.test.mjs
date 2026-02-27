import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createStreamCreditMessage,
  createStreamInitMessage,
  createStreamBatchMessage,
  isStreamBatchMessage,
  normalizeWorkerError,
  resolveDeltaBatchGap,
  resolveSequenceGap,
  shouldResyncFromBatch,
  shouldScheduleGapResync,
} from './stream-protocol.js';

test('createStreamInitMessage validates and normalizes init config', () => {
  const message = createStreamInitMessage({
    apiBase: 'http://127.0.0.1:3000',
    afterSeqId: 15,
    batchWindowMs: 120,
    streamBatchLimit: 10,
    streamIntervalMs: 200,
    initialCredit: 2,
  });

  assert.equal(message.type, 'stream:init');
  assert.equal(message.apiBase, 'http://127.0.0.1:3000');
  assert.equal(message.afterSeqId, 15);
  assert.equal(message.batchWindowMs, 120);
  assert.equal(message.streamBatchLimit, 10);
  assert.equal(message.initialCredit, 2);
});

test('createStreamCreditMessage normalizes credit amount', () => {
  const message = createStreamCreditMessage(0);
  assert.equal(message.type, 'stream:credit');
  assert.equal(message.amount, 1);
});

test('createStreamBatchMessage returns the expected delta schema', () => {
  const message = createStreamBatchMessage({
    latestSeqId: 77,
    transactions: [
      {
        hash: '0xabc',
        sender: '0xdef',
        nonce: 7,
        tx_type: 2,
        seen_unix_ms: 123456,
        source_id: 'rpc-eth-mainnet',
      },
    ],
    hasGap: true,
    sawHello: false,
  });

  assert.equal(isStreamBatchMessage(message), true);
  assert.equal(message.transactions.length, 1);
  assert.equal(message.transactions[0].hash, '0xabc');
  assert.equal(message.latestSeqId, 77);
  assert.equal(message.hasGap, true);
  assert.equal(message.sawHello, false);
});

test('resolveSequenceGap marks discontinuity between sequence ids', () => {
  assert.equal(resolveSequenceGap(100, 101), false);
  assert.equal(resolveSequenceGap(100, 104), true);
  assert.equal(resolveSequenceGap(0, 1), false);
});

test('resolveDeltaBatchGap only checks seq_start continuity', () => {
  assert.equal(resolveDeltaBatchGap(100, 101, false), false);
  assert.equal(resolveDeltaBatchGap(100, 110, false), true);
  assert.equal(resolveDeltaBatchGap(100, 101, true), true);
});

test('shouldResyncFromBatch only resyncs on explicit gap flags', () => {
  assert.equal(shouldResyncFromBatch(100, 120, false), false);
  assert.equal(shouldResyncFromBatch(100, 120, true), true);
  assert.equal(shouldResyncFromBatch(120, 120, true), false);
  assert.equal(shouldResyncFromBatch(120, 119, true), false);
});

test('shouldScheduleGapResync enforces cooldown for repeated gap signals', () => {
  const previousSeqId = 100;
  const latestSeqId = 120;
  const nowUnixMs = 20_000;
  const cooldownMs = 10_000;

  assert.equal(
    shouldScheduleGapResync({
      previousSeqId,
      latestSeqId,
      hasGap: false,
      nowUnixMs,
      lastResyncUnixMs: 0,
      cooldownMs,
    }),
    false,
  );
  assert.equal(
    shouldScheduleGapResync({
      previousSeqId,
      latestSeqId,
      hasGap: true,
      nowUnixMs,
      lastResyncUnixMs: 15_001,
      cooldownMs,
    }),
    false,
  );
  assert.equal(
    shouldScheduleGapResync({
      previousSeqId,
      latestSeqId,
      hasGap: true,
      nowUnixMs,
      lastResyncUnixMs: 9_999,
      cooldownMs,
    }),
    true,
  );
});

test('normalizeWorkerError returns stable message payload', () => {
  const normalized = normalizeWorkerError({
    message: 'socket closed',
    filename: 'worker.js',
    lineno: 30,
    colno: 7,
  });

  assert.equal(normalized.type, 'stream:error');
  assert.match(normalized.message, /socket closed/i);
  assert.match(normalized.detail, /worker\.js:30:7/);
});
