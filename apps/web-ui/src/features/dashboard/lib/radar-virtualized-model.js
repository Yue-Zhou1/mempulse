const DEFAULT_FIXED_TICKER_SIZE = 50;

function normalizeFixedSize(size) {
  const parsed = Number(size);
  if (!Number.isFinite(parsed)) {
    return DEFAULT_FIXED_TICKER_SIZE;
  }
  return Math.min(DEFAULT_FIXED_TICKER_SIZE, Math.max(1, Math.floor(parsed)));
}

export function buildVirtualizedTickerRows(rows, fixedSize = DEFAULT_FIXED_TICKER_SIZE) {
  const normalizedRows = Array.isArray(rows) ? rows : [];
  const size = normalizeFixedSize(fixedSize);
  const models = [];
  for (let index = 0; index < size; index += 1) {
    const row = normalizedRows[index] ?? null;
    models.push({
      key: `slot-${index}`,
      hash: row?.hash ?? null,
      row,
      index,
    });
  }
  return models;
}

export function resolveVirtualizedSelectionIndex(rows, selectedHash, visibleOffset = 0) {
  if (!Array.isArray(rows) || !selectedHash) {
    return -1;
  }
  const startOffset = Number.isFinite(visibleOffset)
    ? Math.max(0, Math.floor(visibleOffset))
    : 0;
  for (let index = 0; index < rows.length; index += 1) {
    if (rows[index]?.hash === selectedHash) {
      return startOffset + index;
    }
  }
  return -1;
}
