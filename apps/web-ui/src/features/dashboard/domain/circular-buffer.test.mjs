import test from 'node:test';
import assert from 'node:assert/strict';
import { createCircularBuffer } from './circular-buffer.js';

test('circular buffer keeps deterministic eviction order at fixed capacity', () => {
  const buffer = createCircularBuffer(3);
  buffer.push(1);
  buffer.push(2);
  buffer.push(3);
  buffer.push(4);

  assert.deepEqual(buffer.toArray(), [2, 3, 4]);
  assert.equal(buffer.size(), 3);
  assert.equal(buffer.capacity(), 3);
});

test('circular buffer supports appendMany and clear', () => {
  const buffer = createCircularBuffer(2);
  buffer.appendMany([10, 20, 30]);
  assert.deepEqual(buffer.toArray(), [20, 30]);

  buffer.clear();
  assert.deepEqual(buffer.toArray(), []);
  assert.equal(buffer.size(), 0);
});
