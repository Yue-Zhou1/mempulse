import test from 'node:test';
import assert from 'node:assert/strict';
import {
  aggregateLongTasks,
  calculateDroppedFrameRatio,
  createDashboardPerfMonitor,
  createRollingFpsCalculator,
  reduceHeapSamples,
} from './perf-metrics.js';

test('setup browser api shim provides Worker, rAF and ResizeObserver', () => {
  assert.equal(typeof globalThis.Worker, 'function');
  assert.equal(typeof globalThis.requestAnimationFrame, 'function');
  assert.equal(typeof globalThis.cancelAnimationFrame, 'function');
  assert.equal(typeof globalThis.ResizeObserver, 'function');
});

test('aggregateLongTasks summarizes durations over threshold', () => {
  const summary = aggregateLongTasks([12, 78, 52, 45], 50);

  assert.deepEqual(summary, {
    count: 2,
    totalMs: 130,
    maxMs: 78,
    thresholdMs: 50,
  });
});

test('createRollingFpsCalculator estimates fps from timestamp samples', () => {
  const calculator = createRollingFpsCalculator({ sampleWindowMs: 1000 });
  calculator.addFrameTime(0);
  calculator.addFrameTime(16.6);
  calculator.addFrameTime(33.2);
  calculator.addFrameTime(49.8);

  const snapshot = calculator.snapshot();
  assert.equal(snapshot.frameCount, 4);
  assert.ok(snapshot.fps >= 59 && snapshot.fps <= 61);
});

test('reduceHeapSamples computes latest, peak and delta', () => {
  const heap = reduceHeapSamples([100, 120, 110, 140]);

  assert.deepEqual(heap, {
    sampleCount: 4,
    latestBytes: 140,
    peakBytes: 140,
    averageBytes: 118,
    deltaBytes: 40,
  });
});

test('calculateDroppedFrameRatio returns percentage of frames over budget', () => {
  const ratio = calculateDroppedFrameRatio([10, 15, 20, 35], 16.67);

  assert.equal(ratio.totalFrames, 4);
  assert.equal(ratio.droppedFrames, 2);
  assert.equal(ratio.ratio, 0.5);
});

test('createDashboardPerfMonitor tracks snapshot/detail request pending and queued telemetry', () => {
  const monitor = createDashboardPerfMonitor();

  monitor.markSnapshotDeferred();
  monitor.beginSnapshotRequest();
  monitor.beginDetailRequest();

  let snapshot = monitor.snapshot();
  assert.equal(snapshot.network.snapshot.deferred, 1);
  assert.equal(snapshot.network.snapshot.inFlight, 1);
  assert.equal(snapshot.network.snapshot.peakInFlight, 1);
  assert.equal(snapshot.network.detail.inFlight, 1);

  monitor.finishSnapshotRequest('aborted');
  monitor.finishDetailRequest('completed');

  snapshot = monitor.snapshot();
  assert.equal(snapshot.network.snapshot.aborted, 1);
  assert.equal(snapshot.network.snapshot.inFlight, 0);
  assert.equal(snapshot.network.detail.completed, 1);
  assert.equal(snapshot.network.detail.inFlight, 0);
});
