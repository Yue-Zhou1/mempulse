function isWithinAgeWindow(row, cutoffUnixMs) {
  if (!Number.isFinite(cutoffUnixMs)) {
    return true;
  }
  const seenUnixMs = row?.seen_unix_ms;
  if (!Number.isFinite(seenUnixMs)) {
    return true;
  }
  return seenUnixMs >= cutoffUnixMs;
}

export function mergeTransactionHistory(existingRows, incomingRows, maxItems, options = {}) {
  const cap = Number.isFinite(maxItems) ? Math.max(0, Math.floor(maxItems)) : 0;
  if (cap === 0) {
    return [];
  }

  const nowUnixMs = Number.isFinite(options.nowUnixMs) ? options.nowUnixMs : Date.now();
  const maxAgeMs = Number.isFinite(options.maxAgeMs)
    ? Math.max(0, Math.floor(options.maxAgeMs))
    : Number.POSITIVE_INFINITY;
  const cutoffUnixMs = Number.isFinite(maxAgeMs) ? nowUnixMs - maxAgeMs : Number.NEGATIVE_INFINITY;

  const merged = [];
  const seen = new Set();

  const append = (rows) => {
    for (const row of rows ?? []) {
      if (!isWithinAgeWindow(row, cutoffUnixMs)) {
        continue;
      }
      const hash = row?.hash;
      if (!hash || seen.has(hash)) {
        continue;
      }
      seen.add(hash);
      merged.push(row);
      if (merged.length >= cap) {
        return true;
      }
    }
    return false;
  };

  if (append(incomingRows)) {
    return merged;
  }
  append(existingRows);
  return merged;
}
