import {
  createStreamBatchMessage,
  createStreamCloseMessage,
  createStreamCreditMessage,
  createStreamOpenMessage,
  isStreamInitMessage,
  normalizeWorkerError,
  resolveDeltaBatchGap,
} from './stream-protocol.js';

let socket = null;
let flushTimer = null;
let lastSeqId = 0;
let sawHello = false;
let hasGap = false;
let txQueue = [];
let pendingCredit = 0;
let currentConfig = null;

function resolveStreamUrl(apiBase, afterSeqId, streamBatchLimit, streamIntervalMs) {
  const url = new URL(apiBase, self.location?.href);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = `${url.pathname.replace(/\/+$/, '')}/stream`;
  const params = new URLSearchParams({
    limit: String(streamBatchLimit),
    interval_ms: String(streamIntervalMs),
  });
  if (Number.isFinite(afterSeqId) && afterSeqId > 0) {
    params.set('after', String(afterSeqId));
  }
  url.search = params.toString();
  return url.toString();
}

function clearFlushTimer() {
  if (flushTimer) {
    self.clearInterval(flushTimer);
    flushTimer = null;
  }
}

function cleanupSocket() {
  if (!socket) {
    return;
  }
  const active = socket;
  socket = null;
  active.onopen = null;
  active.onmessage = null;
  active.onclose = null;
  active.onerror = null;
  if (
    active.readyState === WebSocket.OPEN
    || active.readyState === WebSocket.CONNECTING
    || active.readyState === WebSocket.CLOSING
  ) {
    active.close();
  }
}

function flushBatch(maxItems = null) {
  if (!txQueue.length && !sawHello && !hasGap) {
    return;
  }
  const normalizedMaxItems = Number.isFinite(maxItems)
    ? Math.max(1, Math.floor(maxItems))
    : txQueue.length;
  const transactions = txQueue.slice(0, normalizedMaxItems);
  txQueue = txQueue.slice(transactions.length);
  self.postMessage(
    createStreamBatchMessage({
      latestSeqId: lastSeqId,
      transactions,
      hasGap,
      sawHello,
    }),
  );
  sawHello = false;
  hasGap = false;
}

function startFlushTimer(batchWindowMs) {
  clearFlushTimer();
  flushTimer = self.setInterval(() => {
    flushBatch(currentConfig?.streamBatchLimit);
  }, batchWindowMs);
}

function queueCredit(amount) {
  const normalized = Number.isFinite(amount)
    ? Math.max(1, Math.floor(amount))
    : 1;
  pendingCredit += normalized;
}

function flushCreditToSocket() {
  if (!socket || socket.readyState !== WebSocket.OPEN || pendingCredit <= 0) {
    return;
  }
  const amount = pendingCredit;
  pendingCredit = 0;
  socket.send(JSON.stringify({
    type: 'credit',
    amount,
  }));
}

function startStream(config) {
  cleanupSocket();
  clearFlushTimer();
  currentConfig = config;
  txQueue = [];
  sawHello = false;
  hasGap = false;
  pendingCredit = 0;
  lastSeqId = Number.isFinite(config.afterSeqId) ? Math.max(0, config.afterSeqId) : 0;

  const streamUrl = resolveStreamUrl(
    config.apiBase,
    config.afterSeqId,
    config.streamBatchLimit,
    config.streamIntervalMs,
  );
  const nextSocket = new WebSocket(streamUrl);
  socket = nextSocket;
  startFlushTimer(config.batchWindowMs);

  nextSocket.onopen = () => {
    self.postMessage(createStreamOpenMessage());
    queueCredit(config.initialCredit);
    flushCreditToSocket();
  };

  nextSocket.onmessage = (event) => {
    let payload;
    try {
      payload = JSON.parse(event.data);
    } catch {
      return;
    }

    if (payload?.event === 'hello') {
      sawHello = true;
      return;
    }

    if (payload?.event === 'delta_batch') {
      const seqStart = Number(payload?.seq_start);
      const seqEnd = Number(payload?.seq_end);
      if (!Number.isFinite(seqEnd) || seqEnd <= lastSeqId) {
        return;
      }
      hasGap = hasGap
        || resolveDeltaBatchGap(lastSeqId, seqStart, payload?.has_gap);
      lastSeqId = Math.floor(seqEnd);
      if (Array.isArray(payload?.transactions) && payload.transactions.length) {
        for (const tx of payload.transactions) {
          if (!tx?.hash) {
            continue;
          }
          txQueue.push(tx);
        }
      }
      const flushLimit = Number.isFinite(config?.streamBatchLimit)
        ? Math.max(1, Math.floor(config.streamBatchLimit))
        : 10;
      while (txQueue.length >= flushLimit) {
        flushBatch(flushLimit);
      }
      // Keep backend credits flowing independent of main-thread render cadence.
      queueCredit(1);
      flushCreditToSocket();
      return;
    }

    if (typeof payload?.seq_id !== 'number') {
      return;
    }

    const seqId = payload.seq_id;
    if (!Number.isFinite(seqId) || seqId <= 0 || seqId <= lastSeqId) {
      return;
    }

    hasGap = hasGap || resolveSequenceGap(lastSeqId, seqId);
    lastSeqId = seqId;
  };

  nextSocket.onerror = (errorEvent) => {
    self.postMessage(normalizeWorkerError(errorEvent));
  };

  nextSocket.onclose = (closeEvent) => {
    flushBatch();
    self.postMessage(createStreamCloseMessage(closeEvent.code, closeEvent.reason));
  };
}

function stopStream() {
  flushBatch();
  clearFlushTimer();
  cleanupSocket();
}

self.onmessage = (event) => {
  const message = event.data;
  if (message?.type === 'stream:stop') {
    stopStream();
    return;
  }
  if (message?.type === 'stream:credit') {
    queueCredit(createStreamCreditMessage(message.amount).amount);
    flushCreditToSocket();
    return;
  }
  if (!isStreamInitMessage(message)) {
    return;
  }
  startStream(message);
};
