import test from 'node:test';
import assert from 'node:assert/strict';
import config from '../../vite.config.js';

test('vite config dedupes react runtime dependencies', () => {
  const dedupe = config?.resolve?.dedupe ?? [];
  assert.equal(Array.isArray(dedupe), true);
  assert.equal(dedupe.includes('react'), true);
  assert.equal(dedupe.includes('react-dom'), true);
  assert.equal(dedupe.includes('scheduler'), true);
});
