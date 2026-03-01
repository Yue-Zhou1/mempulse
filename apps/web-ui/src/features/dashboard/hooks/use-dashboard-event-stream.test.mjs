import test from 'node:test';
import assert from 'node:assert/strict';
import { isStreamBatchMessage } from '../workers/stream-protocol.js';
import {
  connectDashboardEventSource,
  disconnectDashboardEventSource,
} from './use-dashboard-event-stream.js';

function buildDeltaPayload(seq = 41) {
  return {
    op: 'DISPATCH',
    type: 'DELTA_BATCH',
    seq,
    channel: 'tx.main',
    has_gap: false,
    patch: {
      upsert: [{
        hash: '0xabc',
        sender: '0xdef',
        nonce: 1,
        tx_type: 2,
        seen_unix_ms: 1_700_000_000_000,
        source_id: 'mock',
      }],
      remove: [],
      feature_upsert: [],
      opportunity_upsert: [],
    },
    watermark: {
      latest_ingest_seq: seq,
    },
    market_stats: {
      total_signal_volume: 100,
      total_tx_count: 100,
      low_risk_count: 90,
      medium_risk_count: 8,
      high_risk_count: 2,
      success_rate_bps: 9_800,
    },
  };
}

test('connectDashboardEventSource opens EventSource with after query', () => {
  const sourceRef = { current: null };

  connectDashboardEventSource({
    sourceRef,
    apiBase: 'http://127.0.0.1:3000',
    afterSeqId: 42,
  });

  const source = sourceRef.current;
  assert.ok(source);
  assert.match(source.url, /\/dashboard\/events-v1/);
  assert.match(source.url, /after=42/);

  disconnectDashboardEventSource(source);
});

test('connectDashboardEventSource emits callbacks for open delta reset and error', () => {
  const sourceRef = { current: null };
  let openCount = 0;
  const batches = [];
  const resets = [];
  const errors = [];

  connectDashboardEventSource({
    sourceRef,
    apiBase: 'http://127.0.0.1:3000',
    afterSeqId: 0,
    onOpen: () => {
      openCount += 1;
    },
    onBatch: (batch) => {
      batches.push(batch);
    },
    onReset: (reset) => {
      resets.push(reset);
    },
    onError: (error) => {
      errors.push(error);
    },
  });

  const source = sourceRef.current;
  source.emitOpen();
  source.emitEvent('delta', JSON.stringify(buildDeltaPayload(55)));
  source.emitEvent('reset', JSON.stringify({ reason: 'gap', latestSeqId: 55 }));
  source.emitError({ message: 'boom' });

  assert.equal(openCount, 1);
  assert.equal(batches.length, 1);
  assert.equal(isStreamBatchMessage(batches[0]), true);
  assert.equal(batches[0].latestSeqId, 55);
  assert.equal(resets.length, 1);
  assert.deepEqual(resets[0], { reason: 'gap', latestSeqId: 55 });
  assert.equal(errors.length, 1);
  assert.equal(errors[0].message, 'boom');

  disconnectDashboardEventSource(source);
});

test('disconnectDashboardEventSource closes source and clears handlers', () => {
  const sourceRef = { current: null };
  connectDashboardEventSource({
    sourceRef,
    apiBase: 'http://127.0.0.1:3000',
    afterSeqId: 0,
  });

  const source = sourceRef.current;
  disconnectDashboardEventSource(source);

  assert.equal(source.closed, true);
  assert.equal(source.onopen, null);
  assert.equal(source.onerror, null);
});
