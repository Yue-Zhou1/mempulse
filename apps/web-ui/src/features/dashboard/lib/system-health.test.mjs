import test from 'node:test';
import assert from 'node:assert/strict';
import { createSystemHealthMonitor } from './system-health.js';

test('system health enters and exits sampling mode based on frame deltas', () => {
  const monitor = createSystemHealthMonitor({
    samplingLagThresholdMs: 30,
    samplingStride: 5,
    samplingFlushIdleMs: 500,
  });

  monitor.recordFrameDelta(35);
  assert.equal(monitor.isSamplingMode(), true);

  for (let index = 0; index < 5; index += 1) {
    monitor.recordFrameDelta(10);
  }
  assert.equal(monitor.isSamplingMode(), false);
});

test('system health triggers emergency purge on heap threshold breach', () => {
  let purged = 0;
  const monitor = createSystemHealthMonitor({
    heapEmergencyPurgeMb: 400,
    onEmergencyPurge: () => {
      purged += 1;
    },
  });

  monitor.recordHeapBytes(420 * 1024 * 1024);
  assert.equal(purged, 1);
});

test('sampling mode schedules trailing flush after idle window', () => {
  let nextTimeoutId = 1;
  const scheduled = [];
  const monitor = createSystemHealthMonitor({
    samplingLagThresholdMs: 30,
    samplingStride: 5,
    samplingFlushIdleMs: 500,
    setTimeoutFn: (callback, delay) => {
      const id = nextTimeoutId;
      nextTimeoutId += 1;
      scheduled.push({ id, callback, delay });
      return id;
    },
    clearTimeoutFn: () => {},
  });

  monitor.recordFrameDelta(32);
  assert.equal(monitor.isSamplingMode(), true);
  assert.equal(monitor.shouldRenderBatch(1), false);

  let flushed = 0;
  monitor.scheduleTrailingFlush(() => {
    flushed += 1;
  });
  assert.equal(scheduled.length, 1);
  assert.equal(scheduled[0].delay, 500);

  scheduled[0].callback();
  assert.equal(flushed, 1);
});
