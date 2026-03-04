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
  let onUpdate = null;
  let onComplete = null;
  let from = 0;
  let to = 0;
  let currentValue = 0;
  let duration = 520;
  let startTs = null;

  const resolveDuration = (durationMs, fallback = 520) => (
    Number.isFinite(durationMs)
      ? Math.max(120, Math.floor(durationMs))
      : fallback
  );

  const completeImmediately = () => {
    currentValue = to;
    onUpdate?.(to, 1);
    onComplete?.(to);
  };

  const tick = (timestampMs) => {
    if (!running) {
      return;
    }
    const frameTs = Number.isFinite(timestampMs) ? timestampMs : now();
    if (startTs == null) {
      startTs = frameTs;
    }
    const progress = Math.min(1, (frameTs - startTs) / duration);
    const nextValue = from + (to - from) * easing(progress);
    currentValue = nextValue;
    onUpdate?.(nextValue, progress);
    if (progress < 1) {
      frameHandle = requestFrame(tick);
      return;
    }
    frameHandle = null;
    running = false;
    onComplete?.(to);
  };

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
    onUpdate: nextOnUpdate,
    onComplete: nextOnComplete,
  }) => {
    stop();
    onUpdate = typeof nextOnUpdate === 'function' ? nextOnUpdate : null;
    onComplete = typeof nextOnComplete === 'function' ? nextOnComplete : null;
    from = normalizeFinite(fromValue, 0);
    to = normalizeFinite(toValue, 0);
    currentValue = from;
    duration = resolveDuration(durationMs);
    startTs = null;

    if (!requestFrame || to === from) {
      completeImmediately();
      return stop;
    }

    running = true;
    frameHandle = requestFrame(tick);
    return stop;
  };

  const retarget = ({
    toValue,
    durationMs,
    onUpdate: nextOnUpdate,
    onComplete: nextOnComplete,
  }) => {
    if (typeof nextOnUpdate === 'function') {
      onUpdate = nextOnUpdate;
    }
    if (typeof nextOnComplete === 'function') {
      onComplete = nextOnComplete;
    }

    const nextTo = normalizeFinite(toValue, currentValue);
    const nextDuration = resolveDuration(durationMs, duration);
    if (!requestFrame || nextTo === currentValue) {
      stop();
      to = nextTo;
      duration = nextDuration;
      completeImmediately();
      return stop;
    }

    from = currentValue;
    to = nextTo;
    duration = nextDuration;
    startTs = null;
    if (!running) {
      running = true;
      frameHandle = requestFrame(tick);
    }
    return stop;
  };

  return {
    start,
    retarget,
    stop,
    isRunning() {
      return running;
    },
  };
}
