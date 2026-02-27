const DEFAULT_FIXED_TICKER_SIZE = 50;

function normalizeFixedSize(size) {
  const parsed = Number(size);
  if (!Number.isFinite(parsed)) {
    return DEFAULT_FIXED_TICKER_SIZE;
  }
  return Math.min(DEFAULT_FIXED_TICKER_SIZE, Math.max(1, Math.floor(parsed)));
}

export function buildVirtualizedTickerRows(
  rows,
  fixedSize = DEFAULT_FIXED_TICKER_SIZE,
  previousModels = null,
) {
  const normalizedRows = Array.isArray(rows) ? rows : [];
  const size = normalizeFixedSize(fixedSize);
  const previous = Array.isArray(previousModels) && previousModels.length === size
    ? previousModels
    : null;
  const models = [];
  let reusedCount = 0;
  for (let index = 0; index < size; index += 1) {
    const row = normalizedRows[index] ?? null;
    const hash = row?.hash ?? null;
    const previousModel = previous?.[index] ?? null;
    if (
      previousModel
      && previousModel.index === index
      && previousModel.key === `slot-${index}`
      && previousModel.hash === hash
      && previousModel.row === row
    ) {
      models.push(previousModel);
      reusedCount += 1;
      continue;
    }
    models.push({
      key: `slot-${index}`,
      hash,
      row,
      index,
    });
  }
  return previous && reusedCount === size ? previous : models;
}

export function rowsFromVirtualizedModels(rowModels) {
  if (!Array.isArray(rowModels)) {
    return [];
  }
  const rows = [];
  for (const model of rowModels) {
    if (model?.row) {
      rows.push(model.row);
    }
  }
  return rows;
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
