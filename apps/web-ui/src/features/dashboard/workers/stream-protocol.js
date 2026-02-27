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

function normalizeMarketStats(rawStats) {
  if (!rawStats || typeof rawStats !== 'object') {
    return null;
  }
  return {
    total_signal_volume: normalizeInteger(rawStats.total_signal_volume, 0, 0),
    total_tx_count: normalizeInteger(rawStats.total_tx_count, 0, 0),
    low_risk_count: normalizeInteger(rawStats.low_risk_count, 0, 0),
    medium_risk_count: normalizeInteger(rawStats.medium_risk_count, 0, 0),
    high_risk_count: normalizeInteger(rawStats.high_risk_count, 0, 0),
    success_rate_bps: normalizeInteger(rawStats.success_rate_bps, 0, 0),
  };
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
  featureRows = [],
  opportunityRows = [],
  hasGap = false,
  sawHello = false,
  marketStats = null,
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
  const normalizedFeatureRows = (Array.isArray(featureRows) ? featureRows : [])
    .map((row) => {
      const normalizedChainId = Number(row?.chain_id);
      return {
        hash: String(row?.hash ?? ''),
        protocol: String(row?.protocol ?? ''),
        category: String(row?.category ?? ''),
        chain_id: Number.isFinite(normalizedChainId)
          ? Math.max(0, Math.floor(normalizedChainId))
          : null,
        mev_score: normalizeInteger(row?.mev_score, 0, 0),
        urgency_score: normalizeInteger(row?.urgency_score, 0, 0),
        method_selector: row?.method_selector == null ? null : String(row.method_selector),
        feature_engine_version: String(row?.feature_engine_version ?? ''),
      };
    })
    .filter((row) => row.hash.length > 0);
  const normalizedOpportunityRows = (Array.isArray(opportunityRows) ? opportunityRows : [])
    .map((row) => {
      const normalizedChainId = Number(row?.chain_id);
      const reasons = Array.isArray(row?.reasons)
        ? row.reasons.map((reason) => String(reason ?? '')).filter((reason) => reason.length > 0)
        : [];
      return {
        tx_hash: String(row?.tx_hash ?? ''),
        status: String(row?.status ?? ''),
        strategy: String(row?.strategy ?? ''),
        score: normalizeInteger(row?.score, 0, 0),
        protocol: String(row?.protocol ?? ''),
        category: String(row?.category ?? ''),
        chain_id: Number.isFinite(normalizedChainId)
          ? Math.max(0, Math.floor(normalizedChainId))
          : null,
        feature_engine_version: String(row?.feature_engine_version ?? ''),
        scorer_version: String(row?.scorer_version ?? ''),
        strategy_version: String(row?.strategy_version ?? ''),
        reasons,
        detected_unix_ms: normalizeInteger(row?.detected_unix_ms, 0, 0),
      };
    })
    .filter((row) => row.tx_hash.length > 0);
  return {
    type: 'stream:batch',
    latestSeqId: normalizeSeqId(latestSeqId),
    transactions: normalizedTransactions,
    featureRows: normalizedFeatureRows,
    opportunityRows: normalizedOpportunityRows,
    hasGap: Boolean(hasGap),
    sawHello: Boolean(sawHello),
    marketStats: normalizeMarketStats(marketStats),
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
    && Array.isArray(message?.featureRows)
    && Array.isArray(message?.opportunityRows)
    && typeof message?.hasGap === 'boolean'
    && typeof message?.sawHello === 'boolean'
    && (message?.marketStats == null || typeof message?.marketStats === 'object')
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
  void previousSeqId;
  void seqStart;
  return Boolean(hasGapFlag);
}

export function appendTransactionsWithCap(existingQueue, incomingTransactions, maxItems) {
  const queue = Array.isArray(existingQueue) ? [...existingQueue] : [];
  const incoming = Array.isArray(incomingTransactions) ? incomingTransactions : [];
  for (const row of incoming) {
    const primaryKey = String(row?.hash ?? row?.tx_hash ?? '');
    if (!primaryKey) {
      continue;
    }
    queue.push(row);
  }

  const cap = Number.isFinite(maxItems) ? Math.max(1, Math.floor(maxItems)) : queue.length;
  if (queue.length <= cap) {
    return { queue, dropped: false };
  }
  return {
    queue: queue.slice(queue.length - cap),
    dropped: true,
  };
}

export function shouldResyncFromBatch(previousSeqId, latestSeqId, hasGap = false) {
  const previous = normalizeSeqId(previousSeqId);
  const latest = normalizeSeqId(latestSeqId);
  if (latest <= previous) {
    return false;
  }
  return Boolean(hasGap);
}

export function shouldApplyStreamBatch({
  previousSeqId,
  latestSeqId,
  transactionCount = 0,
  featureCount = 0,
  opportunityCount = 0,
} = {}) {
  const previous = normalizeSeqId(previousSeqId);
  const latest = normalizeSeqId(latestSeqId);
  const count = Number.isFinite(transactionCount)
    ? Math.max(0, Math.floor(transactionCount))
    : 0;
  const features = Number.isFinite(featureCount)
    ? Math.max(0, Math.floor(featureCount))
    : 0;
  const opportunities = Number.isFinite(opportunityCount)
    ? Math.max(0, Math.floor(opportunityCount))
    : 0;
  if (latest > previous) {
    return true;
  }
  if (latest === previous && (count > 0 || features > 0 || opportunities > 0)) {
    return true;
  }
  return false;
}

export function shouldScheduleGapResync({
  previousSeqId,
  latestSeqId,
  hasGap = false,
  nowUnixMs = Date.now(),
  lastResyncUnixMs = 0,
  cooldownMs = 10_000,
} = {}) {
  if (!shouldResyncFromBatch(previousSeqId, latestSeqId, hasGap)) {
    return false;
  }
  const now = Number.isFinite(nowUnixMs) ? Math.floor(nowUnixMs) : Date.now();
  const last = Number.isFinite(lastResyncUnixMs) ? Math.floor(lastResyncUnixMs) : 0;
  const cooldown = Number.isFinite(cooldownMs) ? Math.max(0, Math.floor(cooldownMs)) : 0;
  return now - last >= cooldown;
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
