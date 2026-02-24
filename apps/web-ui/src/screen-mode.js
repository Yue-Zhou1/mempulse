export const screenIds = ['radar', 'opps', 'replay'];

export function normalizeScreenId(value) {
  const normalized = String(value ?? '')
    .trim()
    .toLowerCase();
  if (screenIds.includes(normalized)) {
    return normalized;
  }
  return 'radar';
}
