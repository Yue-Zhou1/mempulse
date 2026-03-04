function normalizeDuration(durationMs, fallback = 520) {
  if (!Number.isFinite(durationMs)) {
    return fallback;
  }
  return Math.max(120, Math.floor(durationMs));
}

function isDigitChar(char) {
  return /^[0-9]$/.test(char);
}

export function buildRollingDigitSlots(previousText, nextText) {
  const previous = String(previousText ?? '');
  const next = String(nextText ?? '');
  const slotsFromRight = [];

  for (let offset = 0; offset < next.length; offset += 1) {
    const nextIndex = next.length - 1 - offset;
    const previousIndex = previous.length - 1 - offset;
    const char = next.charAt(nextIndex);
    const previousChar = previousIndex >= 0 ? previous.charAt(previousIndex) : '';
    const isDigit = isDigitChar(char);
    slotsFromRight.push({
      key: `r${offset}-${isDigit ? 'digit' : 'static'}`,
      char,
      previousChar,
      isDigit,
    });
  }

  return slotsFromRight.reverse();
}

export function resolveRollingTransitionDirection(previousValue, nextValue) {
  const previous = Number.isFinite(previousValue) ? Number(previousValue) : 0;
  const next = Number.isFinite(nextValue) ? Number(nextValue) : 0;
  return next < previous ? 'down' : 'up';
}

export function resolveRollingTransitionSpringConfig(durationMs) {
  const duration = normalizeDuration(durationMs);
  if (duration <= 360) {
    return {
      tension: 260,
      friction: 22,
      clamp: true,
    };
  }
  if (duration <= 700) {
    return {
      tension: 220,
      friction: 24,
      clamp: true,
    };
  }
  return {
    tension: 180,
    friction: 26,
    clamp: true,
  };
}
