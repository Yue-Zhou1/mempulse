export function easeOutCubic(progress) {
  return 1 - (1 - progress) ** 3;
}

function normalizeFinite(value, fallback = 0) {
  return Number.isFinite(value) ? Number(value) : fallback;
}

function resolveRequestFrame(options = {}) {
  if (typeof options.requestFrame === 'function') {
    return options.requestFrame;
  }
  if (typeof globalThis?.requestAnimationFrame === 'function') {
    return globalThis.requestAnimationFrame.bind(globalThis);
  }
  return null;
}

function resolveCancelFrame(options = {}) {
  if (typeof options.cancelFrame === 'function') {
    return options.cancelFrame;
  }
  if (typeof globalThis?.cancelAnimationFrame === 'function') {
    return globalThis.cancelAnimationFrame.bind(globalThis);
  }
  return null;
}

function resolveNow(options = {}) {
  if (typeof options.now === 'function') {
    return options.now;
  }
  if (typeof globalThis?.performance?.now === 'function') {
    return () => globalThis.performance.now();
  }
  return () => Date.now();
}

export function createRollingNumberAnimator(options = {}) {
  const requestFrame = resolveRequestFrame(options);
  const cancelFrame = resolveCancelFrame(options);
  const now = resolveNow(options);
  const easing = typeof options.easing === 'function' ? options.easing : easeOutCubic;
  let frameHandle = null;
  let running = false;

  const stop = () => {
    if (frameHandle != null && typeof cancelFrame === 'function') {
      cancelFrame(frameHandle);
    }
    frameHandle = null;
    running = false;
  };

  const start = ({
    fromValue,
    toValue,
    durationMs,
    onUpdate,
    onComplete,
  }) => {
    stop();
    const from = normalizeFinite(fromValue, 0);
    const to = normalizeFinite(toValue, 0);
    const duration = Number.isFinite(durationMs)
      ? Math.max(120, Math.floor(durationMs))
      : 520;

    if (!requestFrame || to <= from) {
      onUpdate?.(to, 1);
      onComplete?.(to);
      return stop;
    }

    running = true;
    let startTs = null;
    const delta = to - from;
    const tick = (timestampMs) => {
      if (!running) {
        return;
      }
      const frameTs = Number.isFinite(timestampMs) ? timestampMs : now();
      if (startTs == null) {
        startTs = frameTs;
      }
      const progress = Math.min(1, (frameTs - startTs) / duration);
      const nextValue = from + delta * easing(progress);
      onUpdate?.(nextValue, progress);
      if (progress < 1) {
        frameHandle = requestFrame(tick);
        return;
      }
      frameHandle = null;
      running = false;
      onComplete?.(to);
    };

    frameHandle = requestFrame(tick);
    return stop;
  };

  return {
    start,
    stop,
    isRunning() {
      return running;
    },
  };
}
