const RADAR_PROFILE = Object.freeze({
  featureLimit: 280,
  oppLimit: 220,
  replayLimit: 160,
});

const OPPS_PROFILE = Object.freeze({
  txLimit: 80,
  featureLimit: 160,
  oppLimit: 600,
  replayLimit: 80,
});

const REPLAY_PROFILE = Object.freeze({
  txLimit: 40,
  featureLimit: 80,
  oppLimit: 120,
  replayLimit: 60,
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
  if (normalized === 'opps' || normalized === 'replay') {
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
      replayLimit: OPPS_PROFILE.replayLimit,
    };
  }

  if (normalizedScreen === 'replay') {
    return {
      txLimit: Math.min(REPLAY_PROFILE.txLimit, boundedSnapshotTxLimit),
      featureLimit: REPLAY_PROFILE.featureLimit,
      oppLimit: REPLAY_PROFILE.oppLimit,
      replayLimit: REPLAY_PROFILE.replayLimit,
    };
  }

  return {
    txLimit: boundedSnapshotTxLimit,
    featureLimit: RADAR_PROFILE.featureLimit,
    oppLimit: RADAR_PROFILE.oppLimit,
    replayLimit: RADAR_PROFILE.replayLimit,
  };
}

export function buildDashboardSnapshotPath({ txLimit, featureLimit, oppLimit, replayLimit }) {
  const params = new URLSearchParams({
    tx_limit: String(txLimit),
    feature_limit: String(featureLimit),
    opp_limit: String(oppLimit),
    replay_limit: String(replayLimit),
  });
  return `/dashboard/snapshot?${params.toString()}`;
}
