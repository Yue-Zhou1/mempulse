import { useCallback, useEffect, useRef } from 'react';
import {
  createStreamStopMessage,
  normalizeWorkerError,
} from '../workers/stream-protocol.js';

export function resolveDashboardStreamTransport(streamTransport) {
  void streamTransport;
  return 'sse';
}

export function shouldUseDashboardStreamWorker(streamTransport) {
  void streamTransport;
  return false;
}

export function disconnectDashboardStreamWorker(worker) {
  if (!worker) {
    return;
  }
  worker.onmessage = null;
  worker.onerror = null;
  worker.onmessageerror = null;
  worker.postMessage(createStreamStopMessage());
  worker.terminate();
}

export function shouldHandleDashboardWorkerEvent(workerRef, worker) {
  return workerRef?.current === worker;
}

export function useDashboardStreamWorker() {
  const workerRef = useRef(null);

  const disconnectWorker = useCallback(() => {
    const worker = workerRef.current;
    if (!worker) {
      return;
    }
    workerRef.current = null;
    disconnectDashboardStreamWorker(worker);
  }, []);

  const connectWorker = useCallback(({
    onOpen,
    onError,
  }) => {
    disconnectWorker();
    onOpen?.();
    onError?.(
      normalizeWorkerError({
        message: 'Dashboard WebSocket worker transport is deprecated. SSE is now the only supported transport.',
      }),
    );
  }, [disconnectWorker]);

  const sendCredit = useCallback((amount = 1) => {
    void amount;
    const worker = workerRef.current;
    if (!worker) {
      return;
    }
  }, []);

  useEffect(() => () => {
    disconnectWorker();
  }, [disconnectWorker]);

  return {
    connectWorker,
    disconnectWorker,
    sendCredit,
  };
}
