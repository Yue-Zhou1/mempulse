const DEFAULT_BATCH_WINDOW_MS = 1000;
const DEFAULT_STREAM_BATCH_LIMIT = 20;
const DEFAULT_STREAM_INTERVAL_MS = 1000;

function normalizeInteger(value, fallback, min = 0) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  return Math.max(min, Math.floor(parsed));
}

function normalizeSeqId(value) {
  return normalizeInteger(value, 0, 0);
}

export function createStreamInitMessage({
  apiBase,
  afterSeqId = 0,
  batchWindowMs = DEFAULT_BATCH_WINDOW_MS,
  streamBatchLimit = DEFAULT_STREAM_BATCH_LIMIT,
  streamIntervalMs = DEFAULT_STREAM_INTERVAL_MS,
  initialCredit = 1,
}) {
  return {
    type: 'stream:init',
    apiBase: String(apiBase ?? ''),
    afterSeqId: normalizeSeqId(afterSeqId),
    batchWindowMs: normalizeInteger(batchWindowMs, DEFAULT_BATCH_WINDOW_MS, 16),
    streamBatchLimit: normalizeInteger(streamBatchLimit, DEFAULT_STREAM_BATCH_LIMIT, 1),
    streamIntervalMs: normalizeInteger(streamIntervalMs, DEFAULT_STREAM_INTERVAL_MS, 16),
    initialCredit: normalizeInteger(initialCredit, 1, 1),
  };
}

export function createStreamStopMessage() {
  return { type: 'stream:stop' };
}

export function createStreamCreditMessage(amount = 1) {
  return {
    type: 'stream:credit',
    amount: normalizeInteger(amount, 1, 1),
  };
}

export function isStreamInitMessage(message) {
  return (
    message?.type === 'stream:init'
    && typeof message?.apiBase === 'string'
    && Number.isFinite(message?.afterSeqId)
  );
}

export function createStreamBatchMessage({
  latestSeqId = 0,
  transactions = [],
  hasGap = false,
  sawHello = false,
}) {
  const normalizedTransactions = (Array.isArray(transactions) ? transactions : [])
    .map((row) => ({
      hash: String(row?.hash ?? ''),
      sender: String(row?.sender ?? ''),
      nonce: normalizeInteger(row?.nonce, 0, 0),
      tx_type: normalizeInteger(row?.tx_type, 0, 0),
      seen_unix_ms: normalizeInteger(row?.seen_unix_ms, 0, 0),
      source_id: String(row?.source_id ?? ''),
    }))
    .filter((row) => row.hash.length > 0);
  return {
    type: 'stream:batch',
    latestSeqId: normalizeSeqId(latestSeqId),
    transactions: normalizedTransactions,
    hasGap: Boolean(hasGap),
    sawHello: Boolean(sawHello),
  };
}

export function createStreamOpenMessage() {
  return { type: 'stream:open' };
}

export function createStreamCloseMessage(code = 1000, reason = '') {
  return {
    type: 'stream:close',
    code: normalizeInteger(code, 1000, 0),
    reason: String(reason ?? ''),
  };
}

export function isStreamBatchMessage(message) {
  return (
    message?.type === 'stream:batch'
    && Number.isFinite(message?.latestSeqId)
    && Array.isArray(message?.transactions)
    && typeof message?.hasGap === 'boolean'
    && typeof message?.sawHello === 'boolean'
  );
}

export function resolveSequenceGap(previousSeqId, nextSeqId) {
  const previous = normalizeSeqId(previousSeqId);
  const next = normalizeSeqId(nextSeqId);
  if (previous <= 0 || next <= 0) {
    return false;
  }
  return next > previous + 1;
}

export function resolveDeltaBatchGap(previousSeqId, seqStart, hasGapFlag = false) {
  if (Boolean(hasGapFlag)) {
    return true;
  }
  return resolveSequenceGap(previousSeqId, seqStart);
}

export function normalizeWorkerError(errorLike) {
  const message = typeof errorLike?.message === 'string' && errorLike.message
    ? errorLike.message
    : 'Unknown worker stream error';
  const filename = errorLike?.filename ? String(errorLike.filename) : '';
  const line = Number.isFinite(errorLike?.lineno) ? errorLike.lineno : null;
  const column = Number.isFinite(errorLike?.colno) ? errorLike.colno : null;
  const location = filename && line != null && column != null
    ? `${filename}:${line}:${column}`
    : filename;
  return {
    type: 'stream:error',
    message,
    detail: location || '',
  };
}
