export const DASHBOARD_STYLE_NEWSPAPER = 'newspaper';
export const DASHBOARD_STYLE_MODERN_FINANCIAL = 'modern-financial';

export const DASHBOARD_STYLE_OPTIONS = Object.freeze([
  Object.freeze({
    id: DASHBOARD_STYLE_NEWSPAPER,
    label: 'Newspaper',
  }),
  Object.freeze({
    id: DASHBOARD_STYLE_MODERN_FINANCIAL,
    label: 'Modern',
  }),
]);

const DASHBOARD_STYLE_SET = new Set(
  DASHBOARD_STYLE_OPTIONS.map((option) => option.id),
);
const DASHBOARD_STYLE_STORAGE_KEY = 'mempulse_ui_style_mode';

function normalizeToken(value) {
  return String(value ?? '').trim().toLowerCase();
}

export function normalizeDashboardStyleMode(value) {
  const normalized = normalizeToken(value);
  if (DASHBOARD_STYLE_SET.has(normalized)) {
    return normalized;
  }
  return DASHBOARD_STYLE_NEWSPAPER;
}

function resolveStorage(storage) {
  if (storage && typeof storage.getItem === 'function' && typeof storage.setItem === 'function') {
    return storage;
  }
  return globalThis?.window?.localStorage ?? null;
}

export function readStoredDashboardStyleMode(storage) {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) {
    return DASHBOARD_STYLE_NEWSPAPER;
  }
  try {
    return normalizeDashboardStyleMode(targetStorage.getItem(DASHBOARD_STYLE_STORAGE_KEY));
  } catch {
    return DASHBOARD_STYLE_NEWSPAPER;
  }
}

export function persistDashboardStyleMode(styleMode, storage) {
  const normalized = normalizeDashboardStyleMode(styleMode);
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) {
    return normalized;
  }
  try {
    targetStorage.setItem(DASHBOARD_STYLE_STORAGE_KEY, normalized);
  } catch {
    // Ignore storage write failures (private mode, blocked storage).
  }
  return normalized;
}

export function applyDashboardStyleMode(styleMode, targetNode) {
  const normalized = normalizeDashboardStyleMode(styleMode);
  const node = targetNode ?? globalThis?.document?.documentElement;
  if (node && typeof node.setAttribute === 'function') {
    node.setAttribute('data-ui-style', normalized);
  }
  return normalized;
}
