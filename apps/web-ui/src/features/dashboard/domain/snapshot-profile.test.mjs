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
  });
});

test('resolveDashboardSnapshotLimits falls back to radar limits for removed replay screen', () => {
  const limits = resolveDashboardSnapshotLimits({
    activeScreen: 'replay',
    snapshotTxLimit: 140,
  });

  assert.deepEqual(limits, {
    txLimit: 140,
    featureLimit: 280,
    oppLimit: 220,
  });
});

test('buildDashboardSnapshotPath encodes snapshot limit params without replay controls', () => {
  const path = buildDashboardSnapshotPath({
    txLimit: 80,
    featureLimit: 160,
    oppLimit: 600,
  });

  assert.equal(
    path,
    '/dashboard/snapshot?tx_limit=80&feature_limit=160&opp_limit=600',
  );
});
