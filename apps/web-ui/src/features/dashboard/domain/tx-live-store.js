function resolveCap(maxItems) {
  if (!Number.isFinite(maxItems)) {
    return 0;
  }
  return Math.max(0, Math.floor(maxItems));
}

function resolveCutoffUnixMs(nowUnixMs, maxAgeMs) {
  const now = Number.isFinite(nowUnixMs) ? nowUnixMs : Date.now();
  const boundedMaxAge = Number.isFinite(maxAgeMs)
    ? Math.max(0, Math.floor(maxAgeMs))
    : Number.POSITIVE_INFINITY;
  if (!Number.isFinite(boundedMaxAge)) {
    return Number.NEGATIVE_INFINITY;
  }
  return now - boundedMaxAge;
}

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

function rowsReferenceEqual(left, right) {
  if (left.length !== right.length) {
    return false;
  }
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) {
      return false;
    }
  }
  return true;
}

export function createTxLiveStore() {
  let byHash = new Map();
  let order = [];
  let rows = [];

  return {
    snapshot(incomingRows, options = {}) {
      const cap = resolveCap(options.maxItems);
      if (cap === 0) {
        const changed = rows.length !== 0;
        byHash = new Map();
        order = [];
        rows = [];
        return { rows, changed };
      }

      const cutoffUnixMs = resolveCutoffUnixMs(options.nowUnixMs, options.maxAgeMs);
      const seen = new Set();
      const nextOrder = [];
      const nextByHash = new Map();

      for (const row of incomingRows ?? []) {
        const hash = row?.hash;
        if (!hash || seen.has(hash) || !isWithinAgeWindow(row, cutoffUnixMs)) {
          continue;
        }
        seen.add(hash);
        const previous = byHash.get(hash);
        const nextRow = previous && txSummaryEquivalent(previous, row) ? previous : row;
        nextOrder.push(hash);
        nextByHash.set(hash, nextRow);
        if (nextOrder.length >= cap) {
          break;
        }
      }

      if (nextOrder.length < cap) {
        for (const hash of order) {
          if (seen.has(hash)) {
            continue;
          }
          const existing = byHash.get(hash);
          if (!existing || !isWithinAgeWindow(existing, cutoffUnixMs)) {
            continue;
          }
          seen.add(hash);
          nextOrder.push(hash);
          nextByHash.set(hash, existing);
          if (nextOrder.length >= cap) {
            break;
          }
        }
      }

      const nextRows = nextOrder.map((hash) => nextByHash.get(hash));
      const changed = !rowsReferenceEqual(rows, nextRows);

      byHash = nextByHash;
      order = nextOrder;
      if (changed) {
        rows = nextRows;
      }

      return { rows, changed };
    },
    rows() {
      return rows;
    },
    reset() {
      byHash = new Map();
      order = [];
      rows = [];
    },
  };
}
