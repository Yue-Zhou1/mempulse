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
    txLimit: 1,
    featureLimit: 1,
    oppLimit: 600,
  });
});

test('resolveDashboardSnapshotLimits minimizes tx and feature payload on opps screen', () => {
  const limits = resolveDashboardSnapshotLimits({
    activeScreen: 'opps',
    snapshotTxLimit: 500,
  });

  assert.deepEqual(limits, {
    txLimit: 1,
    featureLimit: 1,
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

test('resolveDashboardSnapshotLimits keeps feature limit aligned with large radar tx windows', () => {
  const limits = resolveDashboardSnapshotLimits({
    activeScreen: 'radar',
    snapshotTxLimit: 500,
  });

  assert.deepEqual(limits, {
    txLimit: 500,
    featureLimit: 500,
    oppLimit: 220,
  });
});

test('buildDashboardSnapshotPath uses snapshot-v2 by default', () => {
  const path = buildDashboardSnapshotPath({
    txLimit: 80,
    featureLimit: 160,
    oppLimit: 600,
  });

  assert.equal(
    path,
    '/dashboard/snapshot-v2?tx_limit=80&feature_limit=160&opp_limit=600',
  );
});
