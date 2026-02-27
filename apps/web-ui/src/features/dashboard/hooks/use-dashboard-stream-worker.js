import { useCallback, useEffect, useRef } from 'react';
import {
  createStreamCreditMessage,
  createStreamInitMessage,
  createStreamStopMessage,
  isStreamBatchMessage,
  normalizeWorkerError,
} from '../workers/stream-protocol.js';

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
    apiBase,
    afterSeqId,
    batchWindowMs,
    streamBatchLimit,
    streamIntervalMs,
    initialCredit,
    onOpen,
    onBatch,
    onClose,
    onError,
  }) => {
    disconnectWorker();
    if (typeof Worker !== 'function') {
      onError?.(normalizeWorkerError({ message: 'Web Worker is not available in this runtime' }));
      return;
    }

    const worker = new Worker(
      new URL('../workers/dashboard-stream.worker.js', import.meta.url),
      { type: 'module' },
    );
    workerRef.current = worker;

    worker.onmessage = (event) => {
      if (!shouldHandleDashboardWorkerEvent(workerRef, worker)) {
        return;
      }
      const message = event.data;
      if (message?.type === 'stream:open') {
        onOpen?.();
        return;
      }
      if (isStreamBatchMessage(message)) {
        onBatch?.(message);
        return;
      }
      if (message?.type === 'stream:close') {
        onClose?.(message);
        return;
      }
      if (message?.type === 'stream:error') {
        onError?.(message);
      }
    };

    worker.onerror = (errorEvent) => {
      if (!shouldHandleDashboardWorkerEvent(workerRef, worker)) {
        return;
      }
      onError?.(normalizeWorkerError(errorEvent));
    };

    worker.postMessage(
      createStreamInitMessage({
        apiBase,
        afterSeqId,
        batchWindowMs,
        streamBatchLimit,
        streamIntervalMs,
        initialCredit,
      }),
    );
  }, [disconnectWorker]);

  const sendCredit = useCallback((amount = 1) => {
    const worker = workerRef.current;
    if (!worker) {
      return;
    }
    worker.postMessage(createStreamCreditMessage(amount));
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
