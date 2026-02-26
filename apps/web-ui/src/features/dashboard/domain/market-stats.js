const METRIC_FIELDS = Object.freeze([
  Object.freeze({ stateKey: 'totalSignalVolume', rawKey: 'total_signal_volume' }),
  Object.freeze({ stateKey: 'totalTxCount', rawKey: 'total_tx_count' }),
  Object.freeze({ stateKey: 'lowRiskCount', rawKey: 'low_risk_count' }),
  Object.freeze({ stateKey: 'mediumRiskCount', rawKey: 'medium_risk_count' }),
  Object.freeze({ stateKey: 'highRiskCount', rawKey: 'high_risk_count' }),
]);

function toNonNegativeInt(value) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    return 0;
  }
  return Math.max(0, Math.floor(numeric));
}

function toSuccessRate(valueBps, totalTxCount, highRiskCount) {
  const parsedBps = Number(valueBps);
  if (Number.isFinite(parsedBps)) {
    const clampedBps = Math.max(0, Math.min(10_000, Math.floor(parsedBps)));
    return clampedBps / 100;
  }
  if (totalTxCount === 0) {
    return 100;
  }
  const successful = Math.max(0, totalTxCount - highRiskCount);
  return (successful / totalTxCount) * 100;
}

export function createMarketStatsState() {
  return {
    totalSignalVolume: 0,
    totalTxCount: 0,
    lowRiskCount: 0,
    mediumRiskCount: 0,
    highRiskCount: 0,
    successRate: 100,
  };
}

export function resolveMarketStatsSnapshot(rawStats, fallbackState = createMarketStatsState()) {
  const fallback = {
    ...fallbackState,
  };
  if (!rawStats || typeof rawStats !== 'object') {
    return fallback;
  }

  const next = {};
  for (const field of METRIC_FIELDS) {
    next[field.stateKey] = toNonNegativeInt(rawStats[field.rawKey]);
  }
  next.successRate = toSuccessRate(rawStats.success_rate_bps, next.totalTxCount, next.highRiskCount);
  return next;
}
