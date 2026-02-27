const RADAR_PROFILE = Object.freeze({
  featureLimit: 280,
  oppLimit: 220,
});

const OPPS_PROFILE = Object.freeze({
  txLimit: 80,
  featureLimit: 160,
  oppLimit: 600,
});

function clampInt(value, min, max, fallback) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    return fallback;
  }
  const integer = Math.floor(numeric);
  return Math.min(max, Math.max(min, integer));
}

function normalizeScreen(screen) {
  const normalized = String(screen ?? '').trim().toLowerCase();
  if (normalized === 'opps') {
    return normalized;
  }
  return 'radar';
}

export function resolveDashboardSnapshotLimits({ activeScreen, snapshotTxLimit }) {
  const normalizedScreen = normalizeScreen(activeScreen);
  const boundedSnapshotTxLimit = clampInt(snapshotTxLimit, 1, 500, 120);

  if (normalizedScreen === 'opps') {
    return {
      txLimit: Math.min(OPPS_PROFILE.txLimit, boundedSnapshotTxLimit),
      featureLimit: OPPS_PROFILE.featureLimit,
      oppLimit: OPPS_PROFILE.oppLimit,
    };
  }

  return {
    txLimit: boundedSnapshotTxLimit,
    featureLimit: RADAR_PROFILE.featureLimit,
    oppLimit: RADAR_PROFILE.oppLimit,
  };
}

export function buildDashboardSnapshotPath({ txLimit, featureLimit, oppLimit }) {
  const params = new URLSearchParams({
    tx_limit: String(txLimit),
    feature_limit: String(featureLimit),
    opp_limit: String(oppLimit),
    replay_limit: '0',
  });
  return `/dashboard/snapshot?${params.toString()}`;
}
