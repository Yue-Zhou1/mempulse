function parseBooleanFlag(rawValue, fallback) {
  if (rawValue == null) {
    return fallback;
  }
  if (typeof rawValue === 'boolean') {
    return rawValue;
  }
  const normalized = String(rawValue).trim().toLowerCase();
  if (!normalized) {
    return fallback;
  }
  if (['1', 'true', 'yes', 'on'].includes(normalized)) {
    return true;
  }
  if (['0', 'false', 'no', 'off'].includes(normalized)) {
    return false;
  }
  return fallback;
}

function resolveEnv(envLike) {
  if (envLike && typeof envLike === 'object') {
    return envLike;
  }
  if (typeof import.meta !== 'undefined' && import.meta?.env) {
    return import.meta.env;
  }
  return {};
}

export function resolveStrictModeEnabled(envLike) {
  const env = resolveEnv(envLike);
  return parseBooleanFlag(env.VITE_UI_STRICT_MODE, true);
}

export function resolveReactDevtoolsHookEnabled(envLike) {
  const env = resolveEnv(envLike);
  return parseBooleanFlag(env.VITE_UI_REACT_DEVTOOLS_HOOK_ENABLED, true);
}
