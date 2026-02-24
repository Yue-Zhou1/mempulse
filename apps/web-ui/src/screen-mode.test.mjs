import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeScreenId, screenIds } from './screen-mode.js';

test('normalizeScreenId defaults to radar for unknown values', () => {
  assert.equal(normalizeScreenId(undefined), 'radar');
  assert.equal(normalizeScreenId(''), 'radar');
  assert.equal(normalizeScreenId('unknown'), 'radar');
});

test('normalizeScreenId accepts radar, opps, and replay', () => {
  assert.deepEqual(screenIds, ['radar', 'opps', 'replay']);
  assert.equal(normalizeScreenId('radar'), 'radar');
  assert.equal(normalizeScreenId('opps'), 'opps');
  assert.equal(normalizeScreenId('replay'), 'replay');
});
