export function mergeTransactionHistory(existingRows, incomingRows, maxItems) {
  const cap = Number.isFinite(maxItems) ? Math.max(0, Math.floor(maxItems)) : 0;
  if (cap === 0) {
    return [];
  }

  const merged = [];
  const seen = new Set();

  const append = (rows) => {
    for (const row of rows) {
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
