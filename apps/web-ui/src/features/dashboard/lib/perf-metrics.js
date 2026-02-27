const DEFAULT_LONG_TASK_THRESHOLD_MS = 50;
const DEFAULT_SAMPLE_LIMIT = 240;

function clampFiniteNumber(value) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return null;
  }
  return parsed;
}

function pushBoundedSample(samples, value, sampleLimit = DEFAULT_SAMPLE_LIMIT) {
  const nextValue = clampFiniteNumber(value);
  if (nextValue == null) {
    return;
  }
  samples.push(nextValue);
  if (samples.length > sampleLimit) {
    samples.splice(0, samples.length - sampleLimit);
  }
}

export function aggregateLongTasks(durationsMs, thresholdMs = DEFAULT_LONG_TASK_THRESHOLD_MS) {
  const threshold = Number.isFinite(thresholdMs) ? thresholdMs : DEFAULT_LONG_TASK_THRESHOLD_MS;
  let count = 0;
  let totalMs = 0;
  let maxMs = 0;

  for (const duration of durationsMs ?? []) {
    const normalized = clampFiniteNumber(duration);
    if (normalized == null || normalized < threshold) {
      continue;
    }
    count += 1;
    totalMs += normalized;
    if (normalized > maxMs) {
      maxMs = normalized;
    }
  }

  return {
    count,
    totalMs: Math.round(totalMs),
    maxMs: Math.round(maxMs),
    thresholdMs: threshold,
  };
}

export function createRollingFpsCalculator({ sampleWindowMs = 1000 } = {}) {
  const sampleWindow = Number.isFinite(sampleWindowMs)
    ? Math.max(16, Math.floor(sampleWindowMs))
    : 1000;
  const frameTimes = [];

  function trimWindow(referenceTimestampMs) {
    const cutoff = referenceTimestampMs - sampleWindow;
    while (frameTimes.length > 0 && frameTimes[0] < cutoff) {
      frameTimes.shift();
    }
  }

  return {
    addFrameTime(timestampMs) {
      const normalized = clampFiniteNumber(timestampMs);
      if (normalized == null) {
        return this.snapshot();
      }
      frameTimes.push(normalized);
      trimWindow(normalized);
      return this.snapshot();
    },
    snapshot() {
      if (frameTimes.length < 2) {
        return {
          fps: 0,
          frameCount: frameTimes.length,
          windowMs: sampleWindow,
        };
      }
      const durationMs = frameTimes[frameTimes.length - 1] - frameTimes[0];
      const fps = durationMs > 0
        ? ((frameTimes.length - 1) * 1000) / durationMs
        : 0;
      return {
        fps,
        frameCount: frameTimes.length,
        windowMs: sampleWindow,
      };
    },
    reset() {
      frameTimes.length = 0;
    },
  };
}

export function reduceHeapSamples(samplesBytes) {
  const normalized = [];
  for (const sample of samplesBytes ?? []) {
    const value = clampFiniteNumber(sample);
    if (value != null) {
      normalized.push(value);
    }
  }
  if (!normalized.length) {
    return {
      sampleCount: 0,
      latestBytes: 0,
      peakBytes: 0,
      averageBytes: 0,
      deltaBytes: 0,
    };
  }
  const latestBytes = normalized[normalized.length - 1];
  const peakBytes = Math.max(...normalized);
  const total = normalized.reduce((sum, value) => sum + value, 0);
  const averageBytes = Math.round(total / normalized.length);
  const deltaBytes = latestBytes - normalized[0];
  return {
    sampleCount: normalized.length,
    latestBytes: Math.round(latestBytes),
    peakBytes: Math.round(peakBytes),
    averageBytes,
    deltaBytes: Math.round(deltaBytes),
  };
}

export function calculateDroppedFrameRatio(frameDurationsMs, frameBudgetMs = 16.67) {
  const budget = Number.isFinite(frameBudgetMs) ? frameBudgetMs : 16.67;
  let totalFrames = 0;
  let droppedFrames = 0;

  for (const duration of frameDurationsMs ?? []) {
    const normalized = clampFiniteNumber(duration);
    if (normalized == null) {
      continue;
    }
    totalFrames += 1;
    if (normalized > budget) {
      droppedFrames += 1;
    }
  }

  return {
    totalFrames,
    droppedFrames,
    ratio: totalFrames > 0 ? droppedFrames / totalFrames : 0,
    frameBudgetMs: budget,
  };
}

export function createDashboardPerfMonitor(globalWindow, options = {}) {
  const sampleLimit = Number.isFinite(options.sampleLimit)
    ? Math.max(16, Math.floor(options.sampleLimit))
    : DEFAULT_SAMPLE_LIMIT;
  const longTaskThresholdMs = Number.isFinite(options.longTaskThresholdMs)
    ? options.longTaskThresholdMs
    : DEFAULT_LONG_TASK_THRESHOLD_MS;
  const frameBudgetMs = Number.isFinite(options.frameBudgetMs)
    ? options.frameBudgetMs
    : 16.67;
  const frameFps = createRollingFpsCalculator();

  const state = {
    snapshotApplyDurationsMs: [],
    transactionCommitDurationsMs: [],
    frameDurationsMs: [],
    heapSamplesBytes: [],
    network: {
      snapshot: {
        started: 0,
        completed: 0,
        failed: 0,
        aborted: 0,
        deferred: 0,
        inFlight: 0,
        peakInFlight: 0,
      },
      detail: {
        started: 0,
        completed: 0,
        failed: 0,
        aborted: 0,
        inFlight: 0,
        peakInFlight: 0,
      },
    },
    updatedAtUnixMs: Date.now(),
  };

  function beginNetworkRequest(target) {
    const bucket = state.network[target];
    if (!bucket) {
      return;
    }
    bucket.started += 1;
    bucket.inFlight += 1;
    bucket.peakInFlight = Math.max(bucket.peakInFlight, bucket.inFlight);
    state.updatedAtUnixMs = Date.now();
  }

  function finishNetworkRequest(target, status = 'completed') {
    const bucket = state.network[target];
    if (!bucket) {
      return;
    }
    if (bucket.inFlight > 0) {
      bucket.inFlight -= 1;
    }
    if (status === 'aborted') {
      bucket.aborted += 1;
    } else if (status === 'failed') {
      bucket.failed += 1;
    } else {
      bucket.completed += 1;
    }
    state.updatedAtUnixMs = Date.now();
  }

  const monitor = {
    recordSnapshotApply(durationMs) {
      pushBoundedSample(state.snapshotApplyDurationsMs, durationMs, sampleLimit);
      state.updatedAtUnixMs = Date.now();
    },
    recordTransactionCommit(durationMs) {
      pushBoundedSample(state.transactionCommitDurationsMs, durationMs, sampleLimit);
      state.updatedAtUnixMs = Date.now();
    },
    recordFrameDuration(durationMs, timestampMs = Date.now()) {
      pushBoundedSample(state.frameDurationsMs, durationMs, sampleLimit);
      frameFps.addFrameTime(timestampMs);
      state.updatedAtUnixMs = Date.now();
    },
    recordHeapSample(heapBytes) {
      pushBoundedSample(state.heapSamplesBytes, heapBytes, sampleLimit);
      state.updatedAtUnixMs = Date.now();
    },
    markSnapshotDeferred() {
      state.network.snapshot.deferred += 1;
      state.updatedAtUnixMs = Date.now();
    },
    beginSnapshotRequest() {
      beginNetworkRequest('snapshot');
    },
    finishSnapshotRequest(status = 'completed') {
      finishNetworkRequest('snapshot', status);
    },
    beginDetailRequest() {
      beginNetworkRequest('detail');
    },
    finishDetailRequest(status = 'completed') {
      finishNetworkRequest('detail', status);
    },
    snapshot() {
      return {
        updatedAtUnixMs: state.updatedAtUnixMs,
        snapshotApply: aggregateLongTasks(
          state.snapshotApplyDurationsMs,
          longTaskThresholdMs,
        ),
        transactionCommit: aggregateLongTasks(
          state.transactionCommitDurationsMs,
          longTaskThresholdMs,
        ),
        frame: {
          ...calculateDroppedFrameRatio(state.frameDurationsMs, frameBudgetMs),
          rollingFps: frameFps.snapshot().fps,
        },
        heap: reduceHeapSamples(state.heapSamplesBytes),
        network: {
          snapshot: { ...state.network.snapshot },
          detail: { ...state.network.detail },
        },
      };
    },
  };

  if (globalWindow && typeof globalWindow === 'object') {
    globalWindow.__MEMPULSE_PERF__ = {
      snapshot: () => monitor.snapshot(),
      markSnapshotDeferred: () => monitor.markSnapshotDeferred(),
      beginSnapshotRequest: () => monitor.beginSnapshotRequest(),
      finishSnapshotRequest: (status) => monitor.finishSnapshotRequest(status),
      beginDetailRequest: () => monitor.beginDetailRequest(),
      finishDetailRequest: (status) => monitor.finishDetailRequest(status),
    };
  }

  return monitor;
}
