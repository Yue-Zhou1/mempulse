import { normalizeScreenId } from './screen-mode.js';

export function resolveDashboardRuntimePolicy(activeScreen) {
  const normalizedScreen = normalizeScreenId(activeScreen);
  const shouldComputeRadarDerived = normalizedScreen === 'radar';
  return {
    shouldProcessTxStream: shouldComputeRadarDerived,
    shouldForceTxWindowSnapshot: shouldComputeRadarDerived,
    shouldComputeRadarDerived,
  };
}
