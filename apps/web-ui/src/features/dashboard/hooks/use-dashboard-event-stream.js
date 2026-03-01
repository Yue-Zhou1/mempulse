import { useCallback, useEffect, useRef } from 'react';
import {
  createStreamBatchMessage,
  normalizeWorkerError,
} from '../workers/stream-protocol.js';

const EVENTS_V1_PATH = '/dashboard/events-v1';
const EVENT_SOURCE_HANDLERS = Symbol('dashboardEventSourceHandlers');

function normalizeSeqId(value, fallback = 0) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  return Math.max(0, Math.floor(parsed));
}

function parseJsonData(data) {
  if (typeof data !== 'string' || !data) {
    return null;
  }
  try {
    return JSON.parse(data);
  } catch {
    return null;
  }
}

export function resolveDashboardEventsV1Url(apiBase, afterSeqId = 0) {
  const url = new URL(String(apiBase ?? ''), globalThis?.window?.location?.href);
  const basePath = url.pathname.replace(/\/+$/, '');
  url.pathname = `${basePath}${EVENTS_V1_PATH}`;
  const normalizedAfter = normalizeSeqId(afterSeqId, 0);
  const params = new URLSearchParams();
  if (normalizedAfter > 0) {
    params.set('after', String(normalizedAfter));
  }
  url.search = params.toString();
  return url.toString();
}

export function disconnectDashboardEventSource(source) {
  if (!source) {
    return;
  }

  source.onopen = null;
  source.onerror = null;

  const handlers = source[EVENT_SOURCE_HANDLERS];
  if (handlers && typeof source.removeEventListener === 'function') {
    if (typeof handlers.delta === 'function') {
      source.removeEventListener('delta', handlers.delta);
    }
    if (typeof handlers.reset === 'function') {
      source.removeEventListener('reset', handlers.reset);
    }
  }
  source[EVENT_SOURCE_HANDLERS] = null;

  if (typeof source.close === 'function') {
    source.close();
  }
}

export function shouldHandleDashboardEventSource(sourceRef, source) {
  return sourceRef?.current === source;
}

export function connectDashboardEventSource({
  sourceRef,
  apiBase,
  afterSeqId,
  onOpen,
  onBatch,
  onReset,
  onError,
  EventSourceImpl = globalThis.EventSource,
} = {}) {
  const currentSource = sourceRef?.current;
  if (currentSource) {
    sourceRef.current = null;
    disconnectDashboardEventSource(currentSource);
  }

  if (typeof EventSourceImpl !== 'function') {
    onError?.(normalizeWorkerError({ message: 'EventSource is not available in this runtime' }));
    return null;
  }

  const source = new EventSourceImpl(resolveDashboardEventsV1Url(apiBase, afterSeqId));
  if (sourceRef) {
    sourceRef.current = source;
  }

  const deltaHandler = (event) => {
    if (!shouldHandleDashboardEventSource(sourceRef, source)) {
      return;
    }
    const payload = parseJsonData(event?.data);
    if (!payload) {
      onError?.(normalizeWorkerError({ message: 'Invalid SSE delta payload' }));
      return;
    }
    const batch = createStreamBatchMessage({
      latestSeqId: payload?.watermark?.latest_ingest_seq ?? payload?.seq ?? 0,
      transactions: payload?.patch?.upsert,
      featureRows: payload?.patch?.feature_upsert,
      opportunityRows: payload?.patch?.opportunity_upsert,
      hasGap: payload?.has_gap,
      sawHello: false,
      marketStats: payload?.market_stats,
    });
    onBatch?.(batch);
  };

  const resetHandler = (event) => {
    if (!shouldHandleDashboardEventSource(sourceRef, source)) {
      return;
    }
    const payload = parseJsonData(event?.data);
    if (!payload) {
      onError?.(normalizeWorkerError({ message: 'Invalid SSE reset payload' }));
      return;
    }
    onReset?.({
      reason: String(payload?.reason ?? 'gap'),
      latestSeqId: normalizeSeqId(payload?.latestSeqId ?? payload?.latest_seq_id, 0),
    });
  };

  source[EVENT_SOURCE_HANDLERS] = {
    delta: deltaHandler,
    reset: resetHandler,
  };

  if (typeof source.addEventListener === 'function') {
    source.addEventListener('delta', deltaHandler);
    source.addEventListener('reset', resetHandler);
  }

  source.onopen = () => {
    if (!shouldHandleDashboardEventSource(sourceRef, source)) {
      return;
    }
    onOpen?.();
  };

  source.onerror = (errorEvent) => {
    if (!shouldHandleDashboardEventSource(sourceRef, source)) {
      return;
    }
    onError?.(normalizeWorkerError(errorEvent));
  };

  return source;
}

export function useDashboardEventStream() {
  const sourceRef = useRef(null);

  const disconnectEventSource = useCallback(() => {
    const source = sourceRef.current;
    if (!source) {
      return;
    }
    sourceRef.current = null;
    disconnectDashboardEventSource(source);
  }, []);

  const connectEventSource = useCallback((config) => {
    connectDashboardEventSource({
      ...config,
      sourceRef,
    });
  }, []);

  useEffect(() => () => {
    disconnectEventSource();
  }, [disconnectEventSource]);

  return {
    connectEventSource,
    disconnectEventSource,
  };
}
