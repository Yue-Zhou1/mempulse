function normalizeApiBase(value) {
  if (!value || typeof value !== 'string') {
    return null;
  }

  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }

  try {
    return new URL(trimmed).origin;
  } catch {
    return null;
  }
}

export function resolveApiBase({
  search,
  storedApiBase,
  protocol,
  hostname,
}) {
  const params = new URLSearchParams(search ?? '');
  const queryApiBase = normalizeApiBase(params.get('apiBase'));
  if (queryApiBase) {
    return {
      apiBase: queryApiBase,
      persistApiBase: queryApiBase,
    };
  }

  const stored = normalizeApiBase(storedApiBase);
  if (stored) {
    return {
      apiBase: stored,
      persistApiBase: null,
    };
  }

  return {
    apiBase: `${protocol}//${hostname}:3000`,
    persistApiBase: null,
  };
}
