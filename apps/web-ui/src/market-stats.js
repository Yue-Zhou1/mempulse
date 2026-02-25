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

  const totalSignalVolume = toNonNegativeInt(rawStats.total_signal_volume);
  const totalTxCount = toNonNegativeInt(rawStats.total_tx_count);
  const lowRiskCount = toNonNegativeInt(rawStats.low_risk_count);
  const mediumRiskCount = toNonNegativeInt(rawStats.medium_risk_count);
  const highRiskCount = toNonNegativeInt(rawStats.high_risk_count);
  const successRate = toSuccessRate(rawStats.success_rate_bps, totalTxCount, highRiskCount);

  return {
    totalSignalVolume,
    totalTxCount,
    lowRiskCount,
    mediumRiskCount,
    highRiskCount,
    successRate,
  };
}
