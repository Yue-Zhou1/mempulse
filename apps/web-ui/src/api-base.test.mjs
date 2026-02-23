import test from 'node:test';
import assert from 'node:assert/strict';
import { resolveApiBase } from './api-base.js';

test('query apiBase overrides default and normalizes origin', () => {
  const resolved = resolveApiBase({
    search: '?apiBase=http://172.20.48.1:3000/replay',
    storedApiBase: null,
    protocol: 'http:',
    hostname: '127.0.0.1',
  });

  assert.equal(resolved.apiBase, 'http://172.20.48.1:3000');
  assert.equal(resolved.persistApiBase, 'http://172.20.48.1:3000');
});

test('query apiBase accepts relative proxy path', () => {
  const resolved = resolveApiBase({
    search: '?apiBase=%2Fapi',
    storedApiBase: null,
    protocol: 'http:',
    hostname: '127.0.0.1',
  });

  assert.equal(resolved.apiBase, '/api');
  assert.equal(resolved.persistApiBase, '/api');
});

test('stored apiBase is used when query is missing', () => {
  const resolved = resolveApiBase({
    search: '',
    storedApiBase: 'http://172.20.48.1:3000',
    protocol: 'http:',
    hostname: '127.0.0.1',
  });

  assert.equal(resolved.apiBase, 'http://172.20.48.1:3000');
  assert.equal(resolved.persistApiBase, null);
});

test('falls back to page host when no override exists', () => {
  const resolved = resolveApiBase({
    search: '',
    storedApiBase: null,
    protocol: 'http:',
    hostname: '127.0.0.1',
  });

  assert.equal(resolved.apiBase, 'http://127.0.0.1:3000');
});
