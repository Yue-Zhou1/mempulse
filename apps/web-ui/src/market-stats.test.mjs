import test from 'node:test';
import assert from 'node:assert/strict';
import { createMarketStatsState, resolveMarketStatsSnapshot } from './market-stats.js';

test('resolveMarketStatsSnapshot returns defaults when payload is missing', () => {
  assert.deepEqual(resolveMarketStatsSnapshot(undefined), createMarketStatsState());
  assert.deepEqual(resolveMarketStatsSnapshot(null), createMarketStatsState());
  const fallback = {
    totalSignalVolume: 10,
    totalTxCount: 11,
    lowRiskCount: 3,
    mediumRiskCount: 4,
    highRiskCount: 5,
    successRate: 55,
  };
  assert.deepEqual(resolveMarketStatsSnapshot(undefined, fallback), fallback);
});

test('resolveMarketStatsSnapshot maps backend fields to render state', () => {
  const stats = resolveMarketStatsSnapshot({
    total_signal_volume: 9_536,
    total_tx_count: 9_536,
    low_risk_count: 9_201,
    medium_risk_count: 286,
    high_risk_count: 49,
    success_rate_bps: 9_949,
  });

  assert.deepEqual(stats, {
    totalSignalVolume: 9_536,
    totalTxCount: 9_536,
    lowRiskCount: 9_201,
    mediumRiskCount: 286,
    highRiskCount: 49,
    successRate: 99.49,
  });
});

test('resolveMarketStatsSnapshot derives success rate when bps is missing', () => {
  const stats = resolveMarketStatsSnapshot({
    total_signal_volume: 120,
    total_tx_count: 100,
    low_risk_count: 80,
    medium_risk_count: 15,
    high_risk_count: 5,
  });

  assert.equal(stats.successRate, 95);
});
