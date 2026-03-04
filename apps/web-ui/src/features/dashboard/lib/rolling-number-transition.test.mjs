import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildRollingDigitSlots,
  resolveRollingTransitionDirection,
  resolveRollingTransitionSpringConfig,
} from './rolling-number-transition.js';

test('resolveRollingTransitionDirection returns down only for descending values', () => {
  assert.equal(resolveRollingTransitionDirection(100, 95), 'down');
  assert.equal(resolveRollingTransitionDirection(95, 95), 'up');
  assert.equal(resolveRollingTransitionDirection(95, 100), 'up');
});

test('resolveRollingTransitionSpringConfig maps duration to stable spring presets', () => {
  assert.deepEqual(resolveRollingTransitionSpringConfig(300), {
    tension: 260,
    friction: 22,
    clamp: true,
  });
  assert.deepEqual(resolveRollingTransitionSpringConfig(520), {
    tension: 220,
    friction: 24,
    clamp: true,
  });
  assert.deepEqual(resolveRollingTransitionSpringConfig(900), {
    tension: 180,
    friction: 26,
    clamp: true,
  });
});

test('buildRollingDigitSlots aligns previous digits from the right and keeps separators static', () => {
  const slots = buildRollingDigitSlots('999', '1,000');
  assert.deepEqual(
    slots.map((slot) => slot.char),
    ['1', ',', '0', '0', '0'],
  );
  assert.deepEqual(
    slots.map((slot) => slot.previousChar),
    ['', '', '9', '9', '9'],
  );
  assert.deepEqual(
    slots.map((slot) => slot.isDigit),
    [true, false, true, true, true],
  );
});
