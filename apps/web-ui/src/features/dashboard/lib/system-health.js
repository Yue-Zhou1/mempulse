const STABLE_FRAME_COUNT_TO_EXIT = 5;

function normalizeInt(value, fallback, min, max) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  const bounded = Math.floor(parsed);
  return Math.min(max, Math.max(min, bounded));
}

export function createSystemHealthMonitor(options = {}) {
  const samplingLagThresholdMs = normalizeInt(
    options.samplingLagThresholdMs,
    30,
    16,
    120,
  );
  const samplingStride = normalizeInt(options.samplingStride, 5, 1, 20);
  const samplingFlushIdleMs = normalizeInt(options.samplingFlushIdleMs, 500, 100, 2000);
  const heapEmergencyPurgeMb = normalizeInt(options.heapEmergencyPurgeMb, 400, 128, 1024);
  const emergencyPurgeThresholdBytes = heapEmergencyPurgeMb * 1024 * 1024;
  const setTimeoutFn = options.setTimeoutFn ?? globalThis?.setTimeout?.bind(globalThis);
  const clearTimeoutFn = options.clearTimeoutFn ?? globalThis?.clearTimeout?.bind(globalThis);

  let samplingMode = false;
  let stableFrameCount = 0;
  let trailingFlushTimer = null;

  function clearTrailingFlush() {
    if (trailingFlushTimer != null && typeof clearTimeoutFn === 'function') {
      clearTimeoutFn(trailingFlushTimer);
    }
    trailingFlushTimer = null;
  }

  function enterSamplingMode() {
    if (samplingMode) {
      return;
    }
    samplingMode = true;
    stableFrameCount = 0;
    options.onSamplingModeChange?.(true);
  }

  function exitSamplingMode() {
    if (!samplingMode) {
      return;
    }
    samplingMode = false;
    stableFrameCount = 0;
    clearTrailingFlush();
    options.onSamplingModeChange?.(false);
  }

  return {
    isSamplingMode() {
      return samplingMode;
    },
    recordFrameDelta(deltaMs) {
      if (!Number.isFinite(deltaMs)) {
        return samplingMode;
      }
      if (deltaMs > samplingLagThresholdMs) {
        enterSamplingMode();
        return samplingMode;
      }
      if (!samplingMode) {
        return false;
      }
      stableFrameCount += 1;
      if (stableFrameCount >= STABLE_FRAME_COUNT_TO_EXIT) {
        exitSamplingMode();
      }
      return samplingMode;
    },
    shouldRenderBatch(batchIndex) {
      if (!samplingMode) {
        return true;
      }
      const normalizedBatchIndex = Number.isFinite(batchIndex)
        ? Math.max(1, Math.floor(batchIndex))
        : 1;
      return normalizedBatchIndex % samplingStride === 0;
    },
    scheduleTrailingFlush(flush) {
      if (!samplingMode || typeof setTimeoutFn !== 'function') {
        return;
      }
      clearTrailingFlush();
      trailingFlushTimer = setTimeoutFn(() => {
        trailingFlushTimer = null;
        flush?.();
      }, samplingFlushIdleMs);
    },
    clearTrailingFlush,
    recordHeapBytes(usedHeapBytes) {
      if (!Number.isFinite(usedHeapBytes)) {
        return false;
      }
      if (usedHeapBytes <= emergencyPurgeThresholdBytes) {
        return false;
      }
      options.onEmergencyPurge?.(usedHeapBytes);
      return true;
    },
  };
}
