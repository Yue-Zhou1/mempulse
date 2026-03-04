const DEFAULT_LONG_TASK_THRESHOLD_MS = 50;
const DEFAULT_SCROLL_HANDLER_LONG_TASK_THRESHOLD_MS = 8;
const DEFAULT_SAMPLE_LIMIT = 240;
const MB = 1024 * 1024;
const HIGH_PAGE_MEMORY_BYTES = 600 * MB;
const HIGH_HEAP_MEMORY_BYTES = 320 * MB;
const RAPID_MEMORY_DELTA_BYTES = 96 * MB;
const STREAM_ROW_BOUNDARY = 550;
const STREAM_BACKLOG_BOUNDARY = 128;
const DETAIL_CACHE_LARGE_BOUNDARY = 128;

function clampFiniteNumber(value) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return null;
  }
  return parsed;
}

function resolveNonNegativeInt(value, fallback = 0) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return fallback;
  }
  return Math.floor(parsed);
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

function summarizeSamples(samplesMs) {
  const normalized = [];
  for (const sample of samplesMs ?? []) {
    const value = clampFiniteNumber(sample);
    if (value != null) {
      normalized.push(value);
    }
  }
  if (!normalized.length) {
    return {
      sampleCount: 0,
      averageMs: 0,
      maxMs: 0,
      p95Ms: 0,
    };
  }
  const total = normalized.reduce((sum, value) => sum + value, 0);
  const max = Math.max(...normalized);
  const sorted = [...normalized].sort((left, right) => left - right);
  const p95Index = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(sorted.length * 0.95) - 1),
  );
  return {
    sampleCount: normalized.length,
    averageMs: Math.round(total / normalized.length),
    maxMs: Math.round(max),
    p95Ms: Math.round(sorted[p95Index]),
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

export function analyzeMemoryProfile({
  heap = {},
  page = {},
  streamState = {},
} = {}) {
  const heapLatestBytes = resolveNonNegativeInt(heap.latestBytes, 0);
  const heapDeltaBytes = resolveNonNegativeInt(heap.deltaBytes, 0);
  const pageLatestBytes = resolveNonNegativeInt(page.latestBytes, 0);
  const pageDeltaBytes = resolveNonNegativeInt(page.deltaBytes, 0);
  const transactionRows = resolveNonNegativeInt(streamState.transactionRows, 0);
  const pendingTransactions = resolveNonNegativeInt(streamState.pendingTransactions, 0);
  const pendingFeatures = resolveNonNegativeInt(streamState.pendingFeatures, 0);
  const pendingOpportunities = resolveNonNegativeInt(streamState.pendingOpportunities, 0);
  const detailCacheSize = resolveNonNegativeInt(streamState.detailCacheSize, 0);
  const pendingTotal = pendingTransactions + pendingFeatures + pendingOpportunities;

  const reasons = [];
  if (pageDeltaBytes >= RAPID_MEMORY_DELTA_BYTES) {
    reasons.push('page-memory-rising-fast');
  }
  if (pageDeltaBytes >= RAPID_MEMORY_DELTA_BYTES && pageDeltaBytes > heapDeltaBytes * 1.5) {
    reasons.push('non-js-memory-dominant');
  }
  if (
    heapDeltaBytes >= RAPID_MEMORY_DELTA_BYTES
    && transactionRows <= STREAM_ROW_BOUNDARY
    && pendingTotal <= 8
  ) {
    reasons.push('heap-growth-with-bounded-tx');
  }
  if (pendingTotal >= STREAM_BACKLOG_BOUNDARY) {
    reasons.push('stream-backlog');
  }
  if (detailCacheSize >= DETAIL_CACHE_LARGE_BOUNDARY) {
    reasons.push('detail-cache-large');
  }

  let level = 'normal';
  if (pageLatestBytes >= HIGH_PAGE_MEMORY_BYTES || heapLatestBytes >= HIGH_HEAP_MEMORY_BYTES) {
    level = 'high';
  } else if (pageDeltaBytes >= RAPID_MEMORY_DELTA_BYTES || heapDeltaBytes >= RAPID_MEMORY_DELTA_BYTES) {
    level = 'watch';
  }

  let summary = 'Memory growth appears stable for current sample window.';
  if (reasons.includes('non-js-memory-dominant')) {
    summary = 'Page memory is growing faster than JS heap; likely DOM/compositor/GPU/process overhead.';
  } else if (reasons.includes('stream-backlog')) {
    summary = 'Pending stream buffers are accumulating faster than flush commits.';
  } else if (reasons.includes('heap-growth-with-bounded-tx')) {
    summary = 'JS heap is rising while tx rows stay bounded; inspect retained objects/caches.';
  } else if (reasons.includes('page-memory-rising-fast')) {
    summary = 'Page memory is rising quickly and needs continued profiling.';
  }

  return {
    level,
    reasons,
    summary,
    heapLatestBytes,
    heapDeltaBytes,
    pageLatestBytes,
    pageDeltaBytes,
    pendingTotal,
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
  const scrollHandlerLongTaskThresholdMs = Number.isFinite(options.scrollHandlerLongTaskThresholdMs)
    ? options.scrollHandlerLongTaskThresholdMs
    : DEFAULT_SCROLL_HANDLER_LONG_TASK_THRESHOLD_MS;
  const frameFps = createRollingFpsCalculator();

  const state = {
    snapshotApplyDurationsMs: [],
    transactionCommitDurationsMs: [],
    scrollHandlerDurationsMs: [],
    scrollCommitLatenciesMs: [],
    frameDurationsMs: [],
    heapSamplesBytes: [],
    pageMemorySamplesBytes: [],
    domNodeSamples: [],
    streamState: {
      transactionRows: 0,
      recentRows: 0,
      pendingTransactions: 0,
      pendingFeatures: 0,
      pendingOpportunities: 0,
      detailCacheSize: 0,
    },
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
    recordScrollHandler(durationMs) {
      pushBoundedSample(state.scrollHandlerDurationsMs, durationMs, sampleLimit);
      state.updatedAtUnixMs = Date.now();
    },
    recordScrollCommitLatency(durationMs) {
      pushBoundedSample(state.scrollCommitLatenciesMs, durationMs, sampleLimit);
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
    recordMemorySample(sample) {
      const normalized = sample && typeof sample === 'object' ? sample : {};
      pushBoundedSample(state.pageMemorySamplesBytes, normalized.totalMemoryBytes, sampleLimit);
      pushBoundedSample(state.domNodeSamples, normalized.domNodeCount, sampleLimit);
      state.streamState = {
        transactionRows: resolveNonNegativeInt(
          normalized.transactionRows,
          state.streamState.transactionRows,
        ),
        recentRows: resolveNonNegativeInt(
          normalized.recentRows,
          state.streamState.recentRows,
        ),
        pendingTransactions: resolveNonNegativeInt(
          normalized.pendingTransactions,
          state.streamState.pendingTransactions,
        ),
        pendingFeatures: resolveNonNegativeInt(
          normalized.pendingFeatures,
          state.streamState.pendingFeatures,
        ),
        pendingOpportunities: resolveNonNegativeInt(
          normalized.pendingOpportunities,
          state.streamState.pendingOpportunities,
        ),
        detailCacheSize: resolveNonNegativeInt(
          normalized.detailCacheSize,
          state.streamState.detailCacheSize,
        ),
      };
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
      const heap = reduceHeapSamples(state.heapSamplesBytes);
      const page = reduceHeapSamples(state.pageMemorySamplesBytes);
      const domNodes = reduceHeapSamples(state.domNodeSamples);
      const memory = {
        heap,
        page,
        domNodes,
        stream: { ...state.streamState },
        analysis: analyzeMemoryProfile({
          heap,
          page,
          streamState: state.streamState,
        }),
      };
      return {
        updatedAtUnixMs: state.updatedAtUnixMs,
        snapshotApply: aggregateLongTasks(
          state.snapshotApplyDurationsMs,
          longTaskThresholdMs,
        ),
        transactionCommit: {
          ...aggregateLongTasks(
            state.transactionCommitDurationsMs,
            longTaskThresholdMs,
          ),
          samples: summarizeSamples(state.transactionCommitDurationsMs),
        },
        scroll: {
          handler: {
            ...aggregateLongTasks(
              state.scrollHandlerDurationsMs,
              scrollHandlerLongTaskThresholdMs,
            ),
            samples: summarizeSamples(state.scrollHandlerDurationsMs),
          },
          commitLatency: {
            ...aggregateLongTasks(
              state.scrollCommitLatenciesMs,
              longTaskThresholdMs,
            ),
            samples: summarizeSamples(state.scrollCommitLatenciesMs),
          },
        },
        frame: {
          ...calculateDroppedFrameRatio(state.frameDurationsMs, frameBudgetMs),
          rollingFps: frameFps.snapshot().fps,
        },
        heap,
        memory,
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
      recordScrollHandler: (durationMs) => monitor.recordScrollHandler(durationMs),
      recordScrollCommitLatency: (durationMs) => monitor.recordScrollCommitLatency(durationMs),
      recordMemorySample: (sample) => monitor.recordMemorySample(sample),
    };
  }

  return monitor;
}
