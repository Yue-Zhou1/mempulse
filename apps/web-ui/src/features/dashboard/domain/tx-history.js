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

function resolveCap(maxItems) {
  if (!Number.isFinite(maxItems)) {
    return 0;
  }
  return Math.max(0, Math.floor(maxItems));
}

function resolveCutoffUnixMs(options) {
  const nowUnixMs = Number.isFinite(options.nowUnixMs) ? options.nowUnixMs : Date.now();
  const maxAgeMs = Number.isFinite(options.maxAgeMs)
    ? Math.max(0, Math.floor(options.maxAgeMs))
    : Number.POSITIVE_INFINITY;
  if (!Number.isFinite(maxAgeMs)) {
    return Number.NEGATIVE_INFINITY;
  }
  return nowUnixMs - maxAgeMs;
}

function txSummaryEquivalent(left, right) {
  return (
    left?.hash === right?.hash
    && left?.sender === right?.sender
    && left?.nonce === right?.nonce
    && left?.tx_type === right?.tx_type
    && left?.seen_unix_ms === right?.seen_unix_ms
    && left?.source_id === right?.source_id
    && left?.chain_id === right?.chain_id
  );
}

function resolveStableRow(existingByHash, row) {
  const hash = row?.hash;
  if (!hash) {
    return row;
  }
  const existing = existingByHash.get(hash);
  if (existing && txSummaryEquivalent(existing, row)) {
    return existing;
  }
  return row;
}

function appendRows(merged, seen, rows, cap, cutoffUnixMs, existingByHash) {
  for (const row of rows ?? []) {
    if (!isWithinAgeWindow(row, cutoffUnixMs)) {
      continue;
    }
    const hash = row?.hash;
    if (!hash || seen.has(hash)) {
      continue;
    }
    seen.add(hash);
    merged.push(resolveStableRow(existingByHash, row));
    if (merged.length >= cap) {
      return true;
    }
  }
  return false;
}

export function mergeTransactionHistory(existingRows, incomingRows, maxItems, options = {}) {
  const cap = resolveCap(maxItems);
  if (cap === 0) {
    return [];
  }

  const cutoffUnixMs = resolveCutoffUnixMs(options);
  const merged = [];
  const seen = new Set();
  const existingByHash = new Map();
  for (const row of existingRows ?? []) {
    if (row?.hash) {
      existingByHash.set(row.hash, row);
    }
  }

  if (appendRows(merged, seen, incomingRows, cap, cutoffUnixMs, existingByHash)) {
    return merged;
  }
  appendRows(merged, seen, existingRows, cap, cutoffUnixMs, existingByHash);
  return merged;
}
