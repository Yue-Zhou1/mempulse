import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveDashboardRuntimePolicy } from './screen-runtime-policy.js';

test('dashboard runtime policy enables tx stream on radar screen', () => {
  assert.deepEqual(resolveDashboardRuntimePolicy('radar'), {
    shouldProcessTxStream: true,
    shouldForceTxWindowSnapshot: true,
    shouldComputeRadarDerived: true,
  });
});

test('dashboard runtime policy disables tx stream on opps screen', () => {
  assert.deepEqual(resolveDashboardRuntimePolicy('opps'), {
    shouldProcessTxStream: false,
    shouldForceTxWindowSnapshot: false,
    shouldComputeRadarDerived: false,
  });
});

test('dashboard runtime policy defaults to radar behavior for unknown screens', () => {
  assert.deepEqual(resolveDashboardRuntimePolicy('anything-else'), {
    shouldProcessTxStream: true,
    shouldForceTxWindowSnapshot: true,
    shouldComputeRadarDerived: true,
  });
});
