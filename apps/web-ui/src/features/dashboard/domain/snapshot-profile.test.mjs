import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildDashboardSnapshotPath,
  resolveDashboardSnapshotLimits,
} from './snapshot-profile.js';

test('resolveDashboardSnapshotLimits returns radar-focused limits', () => {
  const limits = resolveDashboardSnapshotLimits({
    activeScreen: 'radar',
    snapshotTxLimit: 120,
  });

  assert.deepEqual(limits, {
    txLimit: 120,
    featureLimit: 280,
    oppLimit: 220,
    replayLimit: 160,
  });
});

test('resolveDashboardSnapshotLimits returns opps-focused limits', () => {
  const limits = resolveDashboardSnapshotLimits({
    activeScreen: 'opps',
    snapshotTxLimit: 200,
  });

  assert.deepEqual(limits, {
    txLimit: 80,
    featureLimit: 160,
    oppLimit: 600,
    replayLimit: 80,
  });
});

test('resolveDashboardSnapshotLimits returns replay-focused limits', () => {
  const limits = resolveDashboardSnapshotLimits({
    activeScreen: 'replay',
    snapshotTxLimit: 200,
  });

  assert.deepEqual(limits, {
    txLimit: 40,
    featureLimit: 80,
    oppLimit: 120,
    replayLimit: 60,
  });
});

test('buildDashboardSnapshotPath encodes all snapshot limit params', () => {
  const path = buildDashboardSnapshotPath({
    txLimit: 80,
    featureLimit: 160,
    oppLimit: 600,
    replayLimit: 80,
  });

  assert.equal(
    path,
    '/dashboard/snapshot?tx_limit=80&feature_limit=160&opp_limit=600&replay_limit=80',
  );
});
