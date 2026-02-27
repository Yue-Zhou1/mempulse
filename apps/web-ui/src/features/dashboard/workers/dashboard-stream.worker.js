import {
  appendTransactionsWithCap,
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
let heartbeatTimer = null;
let lastSeqId = 0;
let sawHello = false;
let hasGap = false;
let txQueue = [];
let featureQueue = [];
let opportunityQueue = [];
let latestMarketStats = null;
let pendingCredit = 0;
let currentConfig = null;
const MAX_QUEUE_MULTIPLIER = 8;
const MAX_QUEUE_ABSOLUTE = 2_000;
const MAX_FLUSH_BATCHES_PER_DISPATCH = 8;

function resolveStreamUrl(apiBase, afterSeqId, streamBatchLimit, streamIntervalMs) {
  const url = new URL(apiBase, self.location?.href);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  const basePath = url.pathname.replace(/\/+$/, '');
  const streamPath = '/dashboard/stream-v2';
  url.pathname = `${basePath}${streamPath}`;
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

function clearHeartbeatTimer() {
  if (heartbeatTimer) {
    self.clearInterval(heartbeatTimer);
    heartbeatTimer = null;
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
  if (!txQueue.length && !featureQueue.length && !opportunityQueue.length && !sawHello && !hasGap) {
    return;
  }
  const normalizedMaxItems = Number.isFinite(maxItems)
    ? Math.max(1, Math.floor(maxItems))
    : Math.max(txQueue.length, featureQueue.length, opportunityQueue.length);
  const transactions = txQueue.slice(0, normalizedMaxItems);
  txQueue = txQueue.slice(transactions.length);
  const featureRows = featureQueue.slice(0, normalizedMaxItems);
  featureQueue = featureQueue.slice(featureRows.length);
  const opportunityRows = opportunityQueue.slice(0, normalizedMaxItems);
  opportunityQueue = opportunityQueue.slice(opportunityRows.length);
  self.postMessage(
    createStreamBatchMessage({
      latestSeqId: lastSeqId,
      transactions,
      featureRows,
      opportunityRows,
      hasGap,
      sawHello,
      marketStats: latestMarketStats,
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

function startHeartbeatTimer(intervalMs) {
  clearHeartbeatTimer();
  const normalizedInterval = Number.isFinite(intervalMs)
    ? Math.max(1_000, Math.floor(intervalMs))
    : 15_000;
  heartbeatTimer = self.setInterval(() => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return;
    }
    socket.send(JSON.stringify({ op: 'HEARTBEAT' }));
  }, normalizedInterval);
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
    op: 'CREDIT',
    channel_credit: {
      'tx.main': amount,
    },
  }));
}

function resolveQueueCap(config) {
  const streamLimit = Number.isFinite(config?.streamBatchLimit)
    ? Math.max(1, Math.floor(config.streamBatchLimit))
    : 10;
  return Math.min(MAX_QUEUE_ABSOLUTE, streamLimit * MAX_QUEUE_MULTIPLIER);
}

function queueDispatchTransactions(transactions, config) {
  const next = appendTransactionsWithCap(
    txQueue,
    transactions,
    resolveQueueCap(config),
  );
  txQueue = next.queue;
  if (next.dropped) {
    hasGap = true;
  }
}

function queueDispatchFeatureRows(featureRows, config) {
  const next = appendTransactionsWithCap(
    featureQueue,
    featureRows,
    resolveQueueCap(config),
  );
  featureQueue = next.queue;
  if (next.dropped) {
    hasGap = true;
  }
}

function queueDispatchOpportunityRows(opportunityRows, config) {
  const next = appendTransactionsWithCap(
    opportunityQueue,
    opportunityRows,
    resolveQueueCap(config),
  );
  opportunityQueue = next.queue;
  if (next.dropped) {
    hasGap = true;
  }
}

function flushQueueIfNeeded(config) {
  const flushLimit = Number.isFinite(config?.streamBatchLimit)
    ? Math.max(1, Math.floor(config.streamBatchLimit))
    : 10;
  let flushedBatches = 0;
  while (
    (
      txQueue.length >= flushLimit
      || featureQueue.length >= flushLimit
      || opportunityQueue.length >= flushLimit
    )
    && flushedBatches < MAX_FLUSH_BATCHES_PER_DISPATCH
  ) {
    flushBatch(flushLimit);
    flushedBatches += 1;
  }
}

function handleV2Dispatch(payload, config) {
  if (payload?.op !== 'DISPATCH' || payload?.type !== 'DELTA_BATCH') {
    return false;
  }
  const seq = Number(payload?.seq);
  if (!Number.isFinite(seq) || seq <= lastSeqId) {
    return true;
  }

  hasGap = hasGap || resolveDeltaBatchGap(lastSeqId, seq, payload?.has_gap);
  lastSeqId = Math.floor(seq);
  latestMarketStats = payload?.market_stats ?? latestMarketStats;
  queueDispatchTransactions(payload?.patch?.upsert, config);
  queueDispatchFeatureRows(payload?.patch?.feature_upsert, config);
  queueDispatchOpportunityRows(payload?.patch?.opportunity_upsert, config);
  flushQueueIfNeeded(config);
  return true;
}

function startStream(config) {
  cleanupSocket();
  clearFlushTimer();
  clearHeartbeatTimer();
  currentConfig = config;
  txQueue = [];
  featureQueue = [];
  opportunityQueue = [];
  sawHello = false;
  hasGap = false;
  pendingCredit = 0;
  latestMarketStats = null;
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
    nextSocket.send(JSON.stringify({
      op: 'IDENTIFY',
      snapshot_seq: lastSeqId,
      subscriptions: ['tx.main', 'opportunity.main'],
    }));
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

    if (payload?.op === 'HELLO') {
      sawHello = true;
      startHeartbeatTimer(payload?.heartbeat_interval_ms);
      return;
    }

    if (payload?.op === 'HEARTBEAT_ACK') {
      return;
    }

    if (payload?.op === 'RECONNECT' || payload?.op === 'INVALID_SESSION') {
      hasGap = true;
      flushBatch();
      if (nextSocket.readyState === WebSocket.OPEN || nextSocket.readyState === WebSocket.CONNECTING) {
        nextSocket.close();
      }
      return;
    }

    if (handleV2Dispatch(payload, config)) {
      return;
    }
  };

  nextSocket.onerror = (errorEvent) => {
    self.postMessage(normalizeWorkerError(errorEvent));
  };

  nextSocket.onclose = (closeEvent) => {
    clearHeartbeatTimer();
    flushBatch();
    self.postMessage(createStreamCloseMessage(closeEvent.code, closeEvent.reason));
  };
}

function stopStream() {
  flushBatch();
  featureQueue = [];
  opportunityQueue = [];
  txQueue = [];
  clearFlushTimer();
  clearHeartbeatTimer();
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
