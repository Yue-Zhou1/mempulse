import test from 'node:test';
import assert from 'node:assert/strict';
import { createStreamStopMessage } from '../workers/stream-protocol.js';
import {
  disconnectDashboardStreamWorker,
  resolveDashboardStreamTransport,
  shouldUseDashboardStreamWorker,
  shouldHandleDashboardWorkerEvent,
} from './use-dashboard-stream-worker.js';

test('disconnectDashboardStreamWorker clears handlers before stop and terminate', () => {
  const calls = [];
  const worker = {
    onmessage: () => {},
    onerror: () => {},
    onmessageerror: () => {},
    postMessage: (message) => {
      calls.push(['postMessage', message]);
    },
    terminate: () => {
      calls.push(['terminate']);
    },
  };

  disconnectDashboardStreamWorker(worker);

  assert.equal(worker.onmessage, null);
  assert.equal(worker.onerror, null);
  assert.equal(worker.onmessageerror, null);
  assert.deepEqual(calls[0], ['postMessage', createStreamStopMessage()]);
  assert.deepEqual(calls[1], ['terminate']);
});

test('shouldHandleDashboardWorkerEvent ignores stale worker instances', () => {
  const currentWorker = { id: 'current' };
  const staleWorker = { id: 'stale' };
  const workerRef = { current: currentWorker };

  assert.equal(shouldHandleDashboardWorkerEvent(workerRef, currentWorker), true);
  assert.equal(shouldHandleDashboardWorkerEvent(workerRef, staleWorker), false);
});

test('resolveDashboardStreamTransport always resolves to sse', () => {
  assert.equal(resolveDashboardStreamTransport(undefined), 'sse');
  assert.equal(resolveDashboardStreamTransport('unknown'), 'sse');
  assert.equal(resolveDashboardStreamTransport('ws'), 'sse');
  assert.equal(resolveDashboardStreamTransport('WS'), 'sse');
});

test('shouldUseDashboardStreamWorker is disabled for all transports', () => {
  assert.equal(shouldUseDashboardStreamWorker('sse'), false);
  assert.equal(shouldUseDashboardStreamWorker('ws'), false);
});
