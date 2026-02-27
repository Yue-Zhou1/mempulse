import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeScreenId, screenIds } from './screen-mode.js';

test('normalizeScreenId defaults to radar for unknown values', () => {
  for (const value of [undefined, '', 'unknown']) {
    assert.equal(normalizeScreenId(value), 'radar');
  }
});

test('normalizeScreenId accepts radar and opps only', () => {
  assert.deepEqual(screenIds, ['radar', 'opps']);
  assert.equal(normalizeScreenId('radar'), 'radar');
  assert.equal(normalizeScreenId('opps'), 'opps');
  assert.equal(normalizeScreenId('replay'), 'radar');
});

test('normalizeScreenId trims and lowercases input', () => {
  assert.equal(normalizeScreenId('  Radar  '), 'radar');
  assert.equal(normalizeScreenId('OPPS'), 'opps');
});
