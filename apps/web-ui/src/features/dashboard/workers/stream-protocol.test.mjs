import test from 'node:test';
import assert from 'node:assert/strict';
import {
  appendTransactionsWithCap,
  createStreamCreditMessage,
  createStreamInitMessage,
  createStreamBatchMessage,
  isStreamBatchMessage,
  normalizeWorkerError,
  resolveDeltaBatchGap,
  resolveSequenceGap,
  shouldApplyStreamBatch,
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
  assert.equal(message.streamVersion, undefined);
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
    featureRows: [
      {
        hash: '0xabc',
        protocol: 'uniswap-v3',
        category: 'swap',
        chain_id: 1,
        mev_score: 88,
        urgency_score: 64,
        method_selector: '0xa9059cbb',
        feature_engine_version: 'feature-engine.v1',
      },
    ],
    opportunityRows: [
      {
        tx_hash: '0xabc',
        status: 'detected',
        strategy: 'SandwichCandidate',
        score: 12000,
        protocol: 'uniswap-v3',
        category: 'swap',
        chain_id: 1,
        feature_engine_version: 'feature-engine.v1',
        scorer_version: 'scorer.v1',
        strategy_version: 'strategy.sandwich.v1',
        reasons: ['mev_score=90*120'],
        detected_unix_ms: 123460,
      },
    ],
    hasGap: true,
    sawHello: false,
    marketStats: {
      total_signal_volume: 99,
      total_tx_count: 88,
      low_risk_count: 40,
      medium_risk_count: 30,
      high_risk_count: 18,
      success_rate_bps: 9_500,
    },
  });

  assert.equal(isStreamBatchMessage(message), true);
  assert.equal(message.transactions.length, 1);
  assert.equal(message.transactions[0].hash, '0xabc');
  assert.equal(message.featureRows.length, 1);
  assert.equal(message.featureRows[0].hash, '0xabc');
  assert.equal(message.featureRows[0].protocol, 'uniswap-v3');
  assert.equal(message.featureRows[0].category, 'swap');
  assert.equal(message.featureRows[0].chain_id, 1);
  assert.equal(message.featureRows[0].mev_score, 88);
  assert.equal(message.featureRows[0].urgency_score, 64);
  assert.equal(message.opportunityRows.length, 1);
  assert.equal(message.opportunityRows[0].tx_hash, '0xabc');
  assert.equal(message.opportunityRows[0].strategy, 'SandwichCandidate');
  assert.deepEqual(message.opportunityRows[0].reasons, ['mev_score=90*120']);
  assert.equal(message.latestSeqId, 77);
  assert.equal(message.hasGap, true);
  assert.equal(message.sawHello, false);
  assert.equal(message.marketStats.total_tx_count, 88);
  assert.equal(message.marketStats.success_rate_bps, 9500);
});

test('resolveSequenceGap marks discontinuity between sequence ids', () => {
  assert.equal(resolveSequenceGap(100, 101), false);
  assert.equal(resolveSequenceGap(100, 104), true);
  assert.equal(resolveSequenceGap(0, 1), false);
});

test('resolveDeltaBatchGap honors explicit upstream gap flag for stream-v2', () => {
  assert.equal(resolveDeltaBatchGap(100, 101, false), false);
  assert.equal(resolveDeltaBatchGap(100, 110, false), false);
  assert.equal(resolveDeltaBatchGap(100, 101, true), true);
});

test('appendTransactionsWithCap bounds queue growth and keeps newest rows', () => {
  const existing = [
    { hash: '0x1', seen_unix_ms: 1 },
    { hash: '0x2', seen_unix_ms: 2 },
    { hash: '0x3', seen_unix_ms: 3 },
  ];
  const incoming = [
    { hash: '0x4', seen_unix_ms: 4 },
    { hash: '0x5', seen_unix_ms: 5 },
  ];

  const next = appendTransactionsWithCap(existing, incoming, 4);
  assert.equal(next.dropped, true);
  assert.deepEqual(
    next.queue.map((row) => row.hash),
    ['0x2', '0x3', '0x4', '0x5'],
  );
});

test('appendTransactionsWithCap supports tx_hash keyed rows for opportunity deltas', () => {
  const existing = [
    { tx_hash: '0x1', strategy: 'A' },
    { tx_hash: '0x2', strategy: 'B' },
  ];
  const incoming = [
    { tx_hash: '0x3', strategy: 'C' },
    { tx_hash: '0x4', strategy: 'D' },
  ];

  const next = appendTransactionsWithCap(existing, incoming, 3);
  assert.equal(next.dropped, true);
  assert.deepEqual(
    next.queue.map((row) => row.tx_hash),
    ['0x2', '0x3', '0x4'],
  );
});

test('shouldApplyStreamBatch accepts same-seq payload when it still carries tx rows', () => {
  assert.equal(
    shouldApplyStreamBatch({
      previousSeqId: 100,
      latestSeqId: 100,
      transactionCount: 0,
    }),
    false,
  );
  assert.equal(
    shouldApplyStreamBatch({
      previousSeqId: 100,
      latestSeqId: 100,
      transactionCount: 3,
    }),
    true,
  );
  assert.equal(
    shouldApplyStreamBatch({
      previousSeqId: 100,
      latestSeqId: 99,
      transactionCount: 3,
    }),
    false,
  );
  assert.equal(
    shouldApplyStreamBatch({
      previousSeqId: 100,
      latestSeqId: 100,
      transactionCount: 0,
      featureCount: 2,
    }),
    true,
  );
  assert.equal(
    shouldApplyStreamBatch({
      previousSeqId: 100,
      latestSeqId: 100,
      transactionCount: 0,
      featureCount: 0,
      opportunityCount: 2,
    }),
    true,
  );
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
