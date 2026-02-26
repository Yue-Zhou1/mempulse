const defaultScreenId = 'radar';

export const screenIds = Object.freeze([defaultScreenId, 'opps']);

const SCREEN_ID_SET = new Set(screenIds);

function normalizeToken(value) {
  return String(value ?? '').trim().toLowerCase();
}

export function normalizeScreenId(value) {
  const normalized = normalizeToken(value);
  if (SCREEN_ID_SET.has(normalized)) {
    return normalized;
  }
  return defaultScreenId;
}
