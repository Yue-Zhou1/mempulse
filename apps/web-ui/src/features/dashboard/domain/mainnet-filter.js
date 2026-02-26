export const MAINNET_FILTER_ALL = 'all';

const MAINNET_LABELS = Object.freeze([
  'Ethereum',
  'Base',
  'Optimism',
  'Polygon',
]);

export const MAINNET_FILTER_OPTIONS = Object.freeze([
  Object.freeze({ value: MAINNET_FILTER_ALL, label: 'All Mainnets' }),
  ...MAINNET_LABELS.map((label) => Object.freeze({ value: label, label })),
]);

const MAINNET_LABEL_BY_NORMALIZED = new Map(
  MAINNET_LABELS.map((label) => [label.toLowerCase(), label]),
);

function normalizeToken(value) {
  return String(value ?? '').trim().toLowerCase();
}

export function normalizeMainnetFilter(value) {
  const normalized = normalizeToken(value);
  if (!normalized || normalized === MAINNET_FILTER_ALL) {
    return MAINNET_FILTER_ALL;
  }
  return MAINNET_LABEL_BY_NORMALIZED.get(normalized) ?? MAINNET_FILTER_ALL;
}

export function matchesMainnetFilter(mainnetLabel, filterValue) {
  const normalizedFilter = normalizeMainnetFilter(filterValue);
  if (normalizedFilter === MAINNET_FILTER_ALL) {
    return true;
  }
  return normalizeToken(mainnetLabel) === normalizedFilter.toLowerCase();
}

export function filterRowsByMainnet(rows, getMainnetLabel, filterValue) {
  if (!Array.isArray(rows) || rows.length === 0) {
    return [];
  }

  const normalizedFilter = normalizeMainnetFilter(filterValue);
  if (normalizedFilter === MAINNET_FILTER_ALL) {
    return rows;
  }

  return rows.filter((row) => matchesMainnetFilter(getMainnetLabel(row), normalizedFilter));
}
