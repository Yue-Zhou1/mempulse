export const MAINNET_FILTER_ALL = 'all';

export const MAINNET_FILTER_OPTIONS = Object.freeze([
  Object.freeze({ value: MAINNET_FILTER_ALL, label: 'All Mainnets' }),
  Object.freeze({ value: 'Ethereum', label: 'Ethereum' }),
  Object.freeze({ value: 'Base', label: 'Base' }),
  Object.freeze({ value: 'Optimism', label: 'Optimism' }),
  Object.freeze({ value: 'Polygon', label: 'Polygon' }),
]);

const MAINNET_LABEL_BY_NORMALIZED = new Map(
  MAINNET_FILTER_OPTIONS
    .filter((option) => option.value !== MAINNET_FILTER_ALL)
    .map((option) => [option.value.toLowerCase(), option.value]),
);

export function normalizeMainnetFilter(value) {
  const normalized = String(value ?? '').trim().toLowerCase();
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
  return String(mainnetLabel ?? '').trim().toLowerCase() === normalizedFilter.toLowerCase();
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
