import test from 'node:test';
import assert from 'node:assert/strict';
import {
  analyzeMemoryProfile,
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

test('createDashboardPerfMonitor tracks commit and scroll sample stats', () => {
  const monitor = createDashboardPerfMonitor();

  monitor.recordTransactionCommit(8);
  monitor.recordTransactionCommit(16);
  monitor.recordScrollHandler(0.5);
  monitor.recordScrollHandler(1.5);
  monitor.recordScrollCommitLatency(6);
  monitor.recordScrollCommitLatency(18);

  const snapshot = monitor.snapshot();
  assert.equal(snapshot.transactionCommit.samples.sampleCount, 2);
  assert.equal(snapshot.transactionCommit.samples.averageMs, 12);
  assert.equal(snapshot.transactionCommit.samples.maxMs, 16);
  assert.equal(snapshot.scroll.handler.samples.sampleCount, 2);
  assert.equal(snapshot.scroll.handler.samples.maxMs, 2);
  assert.equal(snapshot.scroll.commitLatency.samples.sampleCount, 2);
  assert.equal(snapshot.scroll.commitLatency.samples.p95Ms, 18);
});

test('createDashboardPerfMonitor exposes scroll recorders on window perf api', () => {
  const fakeWindow = {};
  createDashboardPerfMonitor(fakeWindow);

  assert.equal(typeof fakeWindow.__MEMPULSE_PERF__.recordScrollHandler, 'function');
  assert.equal(typeof fakeWindow.__MEMPULSE_PERF__.recordScrollCommitLatency, 'function');
});

test('analyzeMemoryProfile flags non-heap dominant growth when page memory rises faster', () => {
  const analysis = analyzeMemoryProfile({
    heap: {
      latestBytes: 260 * 1024 * 1024,
      deltaBytes: 20 * 1024 * 1024,
    },
    page: {
      latestBytes: 700 * 1024 * 1024,
      deltaBytes: 180 * 1024 * 1024,
    },
    streamState: {
      transactionRows: 500,
      pendingTransactions: 0,
      pendingFeatures: 0,
      pendingOpportunities: 0,
      detailCacheSize: 12,
    },
  });

  assert.equal(analysis.level, 'high');
  assert.equal(analysis.reasons.includes('non-js-memory-dominant'), true);
  assert.equal(analysis.reasons.includes('page-memory-rising-fast'), true);
});

test('createDashboardPerfMonitor snapshots memory samples and stream state', () => {
  const monitor = createDashboardPerfMonitor();
  monitor.recordHeapSample(220 * 1024 * 1024);
  monitor.recordMemorySample({
    totalMemoryBytes: 680 * 1024 * 1024,
    domNodeCount: 1840,
    transactionRows: 500,
    recentRows: 50,
    pendingTransactions: 0,
    pendingFeatures: 0,
    pendingOpportunities: 0,
    detailCacheSize: 8,
  });

  const snapshot = monitor.snapshot();
  assert.equal(snapshot.memory.page.latestBytes, 680 * 1024 * 1024);
  assert.equal(snapshot.memory.domNodes.latestBytes, 1840);
  assert.equal(snapshot.memory.stream.transactionRows, 500);
  assert.equal(snapshot.memory.stream.recentRows, 50);
  assert.equal(snapshot.memory.stream.detailCacheSize, 8);
  assert.equal(typeof snapshot.memory.analysis.summary, 'string');
});
