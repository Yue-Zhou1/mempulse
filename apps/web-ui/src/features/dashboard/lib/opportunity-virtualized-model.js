const DEFAULT_ROW_HEIGHT_PX = 96;
const DEFAULT_OVERSCAN_ROWS = 4;
const DEFAULT_VIEWPORT_HEIGHT_PX = 480;

function normalizePositiveInt(value, fallback, min = 1, max = Number.POSITIVE_INFINITY) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  const integer = Math.floor(parsed);
  return Math.min(max, Math.max(min, integer));
}

export function buildVirtualizedOpportunityWindow(rows, options = {}) {
  const normalizedRows = Array.isArray(rows) ? rows : [];
  const rowHeightPx = normalizePositiveInt(
    options.rowHeightPx,
    DEFAULT_ROW_HEIGHT_PX,
    40,
    240,
  );
  const overscanRows = normalizePositiveInt(
    options.overscanRows,
    DEFAULT_OVERSCAN_ROWS,
    0,
    32,
  );
  const viewportHeight = normalizePositiveInt(
    options.viewportHeight,
    DEFAULT_VIEWPORT_HEIGHT_PX,
    rowHeightPx,
    4_096,
  );
  const scrollTop = normalizePositiveInt(options.scrollTop, 0, 0, Number.MAX_SAFE_INTEGER);

  const totalRowCount = normalizedRows.length;
  const totalHeight = totalRowCount * rowHeightPx;
  if (totalRowCount === 0) {
    return {
      totalRowCount,
      totalHeight: 0,
      startIndex: 0,
      endIndex: 0,
      visibleRows: [],
      paddingTop: 0,
      paddingBottom: 0,
    };
  }

  const visibleCapacity = Math.max(1, Math.ceil(viewportHeight / rowHeightPx));
  const maxBaseStart = Math.max(0, totalRowCount - visibleCapacity);
  const baseStart = Math.min(Math.floor(scrollTop / rowHeightPx), maxBaseStart);
  const startIndex = Math.max(0, baseStart - overscanRows);
  const endIndex = Math.min(
    totalRowCount,
    startIndex + visibleCapacity + overscanRows * 2,
  );

  return {
    totalRowCount,
    totalHeight,
    startIndex,
    endIndex,
    visibleRows: normalizedRows.slice(startIndex, endIndex),
    paddingTop: startIndex * rowHeightPx,
    paddingBottom: Math.max(0, totalHeight - endIndex * rowHeightPx),
  };
}
