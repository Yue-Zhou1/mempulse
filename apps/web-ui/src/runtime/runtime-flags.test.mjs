import test from 'node:test';
import assert from 'node:assert/strict';
import {
  resolveReactDevtoolsHookEnabled,
  resolveStrictModeEnabled,
} from './runtime-flags.js';

test('resolveStrictModeEnabled defaults to true and honors env overrides', () => {
  assert.equal(resolveStrictModeEnabled({}), true);
  assert.equal(resolveStrictModeEnabled({ VITE_UI_STRICT_MODE: 'true' }), true);
  assert.equal(resolveStrictModeEnabled({ VITE_UI_STRICT_MODE: '1' }), true);
  assert.equal(resolveStrictModeEnabled({ VITE_UI_STRICT_MODE: 'false' }), false);
  assert.equal(resolveStrictModeEnabled({ VITE_UI_STRICT_MODE: '0' }), false);
});

test('resolveReactDevtoolsHookEnabled defaults to true and honors env overrides', () => {
  assert.equal(resolveReactDevtoolsHookEnabled({}), true);
  assert.equal(resolveReactDevtoolsHookEnabled({ VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED: 'true' }), true);
  assert.equal(resolveReactDevtoolsHookEnabled({ VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED: 'yes' }), true);
  assert.equal(resolveReactDevtoolsHookEnabled({ VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED: 'false' }), false);
  assert.equal(resolveReactDevtoolsHookEnabled({ VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED: 'off' }), false);
});
