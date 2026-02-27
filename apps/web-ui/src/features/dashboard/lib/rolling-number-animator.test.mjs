import test from 'node:test';
import assert from 'node:assert/strict';
import { createRollingNumberAnimator } from './rolling-number-animator.js';

function createFrameScheduler() {
  let nextHandle = 1;
  const callbacks = new Map();
  return {
    requestFrame(callback) {
      const handle = nextHandle;
      nextHandle += 1;
      callbacks.set(handle, callback);
      return handle;
    },
    cancelFrame(handle) {
      callbacks.delete(handle);
    },
    flushFrame(timestampMs) {
      const pending = Array.from(callbacks.entries());
      callbacks.clear();
      for (const [, callback] of pending) {
        callback(timestampMs);
      }
    },
    pendingCount() {
      return callbacks.size;
    },
  };
}

test('rolling number animator converges to target value', () => {
  const scheduler = createFrameScheduler();
  const updates = [];
  const animator = createRollingNumberAnimator({
    requestFrame: scheduler.requestFrame.bind(scheduler),
    cancelFrame: scheduler.cancelFrame.bind(scheduler),
  });

  animator.start({
    fromValue: 10,
    toValue: 20,
    durationMs: 100,
    onUpdate(value) {
      updates.push(value);
    },
  });

  assert.equal(scheduler.pendingCount(), 1);
  scheduler.flushFrame(0);
  scheduler.flushFrame(50);
  scheduler.flushFrame(120);

  assert.ok(updates.length >= 3);
  assert.equal(Math.round(updates.at(-1)), 20);
  assert.equal(animator.isRunning(), false);
});

test('rolling number animator resolves immediately when target is not greater', () => {
  const scheduler = createFrameScheduler();
  const updates = [];
  const animator = createRollingNumberAnimator({
    requestFrame: scheduler.requestFrame.bind(scheduler),
    cancelFrame: scheduler.cancelFrame.bind(scheduler),
  });

  animator.start({
    fromValue: 30,
    toValue: 12,
    durationMs: 200,
    onUpdate(value) {
      updates.push(value);
    },
  });

  assert.deepEqual(updates, [12]);
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(animator.isRunning(), false);
});

test('rolling number animator stop cancels pending frame', () => {
  const scheduler = createFrameScheduler();
  const animator = createRollingNumberAnimator({
    requestFrame: scheduler.requestFrame.bind(scheduler),
    cancelFrame: scheduler.cancelFrame.bind(scheduler),
  });

  animator.start({
    fromValue: 0,
    toValue: 50,
    durationMs: 400,
    onUpdate() {},
  });

  assert.equal(scheduler.pendingCount(), 1);
  animator.stop();
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(animator.isRunning(), false);
});
