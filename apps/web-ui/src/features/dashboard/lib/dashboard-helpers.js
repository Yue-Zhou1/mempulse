const storageKey = 'vizApiBase';

const MAINNET_LABEL_BY_CHAIN_ID = new Map([
  [1, 'Ethereum'],
  [8453, 'Base'],
  [10, 'Optimism'],
  [137, 'Polygon'],
]);

const MAINNET_ROW_CLASS_BY_LABEL = new Map([
  ['Ethereum', 'bg-[#fffdf7] hover:bg-[#f6f1e4]'],
  ['Base', 'bg-[#f2f8ff] hover:bg-[#e6f1ff]'],
  ['Optimism', 'bg-[#fff1f1] hover:bg-[#ffe3e3]'],
  ['Polygon', 'bg-[#f4f1ff] hover:bg-[#ebe5ff]'],
]);

const DEFAULT_OPPORTUNITY_PALETTE = Object.freeze({
  activeContainer: 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]',
  activeSubtle: 'text-zinc-300',
  idleContainer: 'border-zinc-900 bg-[#fffdf7] text-zinc-900 hover:bg-zinc-100/60',
  idleSubtle: 'text-zinc-700',
});

function parseChainIdValue(chainId) {
  if (Number.isFinite(chainId)) {
    return Number(chainId);
  }
  if (typeof chainId !== 'string') {
    return null;
  }

  const trimmed = chainId.trim();
  if (!trimmed) {
    return null;
  }

  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

function resolveOpportunityPalette(strategy, category) {
  if (strategy.includes('sandwich')) {
    return {
      activeContainer: 'border-[#5f4413] bg-[#8a621f] text-[#fff8ee]',
      activeSubtle: 'text-[#f9e6c8]',
      idleContainer: 'border-[#8a621f] bg-[#fff2dc] text-[#4a3312] hover:bg-[#fae7c8]',
      idleSubtle: 'text-[#6d4c18]',
    };
  }
  if (strategy.includes('arb')) {
    return {
      activeContainer: 'border-[#1f4328] bg-[#2e5f3a] text-[#eef8f0]',
      activeSubtle: 'text-[#d6ecd8]',
      idleContainer: 'border-[#2e5f3a] bg-[#e7f1e8] text-[#1f3c27] hover:bg-[#d9ebdc]',
      idleSubtle: 'text-[#285033]',
    };
  }
  if (strategy.includes('backrun')) {
    return {
      activeContainer: 'border-[#1f3e6a] bg-[#315b8f] text-[#eef5ff]',
      activeSubtle: 'text-[#d7e6ff]',
      idleContainer: 'border-[#315b8f] bg-[#e7eef8] text-[#1f3657] hover:bg-[#dae5f5]',
      idleSubtle: 'text-[#294875]',
    };
  }
  if (strategy.includes('liquidation') || category.includes('liquidation')) {
    return {
      activeContainer: 'border-[#5a2121] bg-[#7a2d2d] text-[#fff2f2]',
      activeSubtle: 'text-[#f7d8d8]',
      idleContainer: 'border-[#7a2d2d] bg-[#fde8e8] text-[#4f1e1e] hover:bg-[#f9dada]',
      idleSubtle: 'text-[#6b2020]',
    };
  }
  return DEFAULT_OPPORTUNITY_PALETTE;
}

export function getStoredApiBase() {
  try {
    return window.localStorage.getItem(storageKey);
  } catch {
    return null;
  }
}

export function setStoredApiBase(value) {
  try {
    if (value) {
      window.localStorage.setItem(storageKey, value);
    }
  } catch {
    // Ignore storage errors for private/incognito contexts.
  }
}

export function shortHex(value, head = 12, tail = 10) {
  if (!value || value.length <= head + tail + 3) {
    return value ?? '-';
  }
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

export function formatTime(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return '-';
  }
  return new Date(unixMs).toLocaleString();
}

export function formatRelativeTime(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return 'unknown';
  }
  const deltaMs = Date.now() - unixMs;
  const deltaSec = Math.max(0, Math.floor(deltaMs / 1000));
  if (deltaSec < 60) {
    return `${deltaSec}s ago`;
  }
  if (deltaSec < 3600) {
    return `${Math.floor(deltaSec / 60)}m ago`;
  }
  if (deltaSec < 86400) {
    return `${Math.floor(deltaSec / 3600)}h ago`;
  }
  return `${Math.floor(deltaSec / 86400)}d ago`;
}

export function formatTickerTime(unixMs) {
  if (!Number.isFinite(unixMs)) {
    return '----/--/-- --:--:--';
  }
  const date = new Date(unixMs);
  const yyyy = String(date.getFullYear());
  const mm = String(date.getMonth() + 1).padStart(2, '0');
  const dd = String(date.getDate()).padStart(2, '0');
  const hh = String(date.getHours()).padStart(2, '0');
  const mi = String(date.getMinutes()).padStart(2, '0');
  const ss = String(date.getSeconds()).padStart(2, '0');
  return `${yyyy}/${mm}/${dd} ${hh}:${mi}:${ss}`;
}

export function resolveMainnetLabel(chainId, sourceId) {
  const normalizedChainId = parseChainIdValue(chainId);
  if (normalizedChainId != null) {
    const knownLabel = MAINNET_LABEL_BY_CHAIN_ID.get(normalizedChainId);
    if (knownLabel) {
      return knownLabel;
    }
    return `Chain ${normalizedChainId}`;
  }

  const normalizedSource = String(sourceId ?? '').toLowerCase();
  if (!normalizedSource) {
    return 'Unknown';
  }
  if (normalizedSource.includes('base')) {
    return 'Base';
  }
  if (normalizedSource.includes('optimism')) {
    return 'Optimism';
  }
  if (normalizedSource.includes('polygon')) {
    return 'Polygon';
  }
  if (normalizedSource.includes('eth') || normalizedSource.includes('ethereum')) {
    return 'Ethereum';
  }
  return 'Unknown';
}

export function resolveMainnetRowClasses(mainnetLabel) {
  return MAINNET_ROW_CLASS_BY_LABEL.get(mainnetLabel) ?? 'bg-[#fffdf7] hover:bg-black/5';
}

export function normalizeChainStatusRows(rawRows) {
  if (!Array.isArray(rawRows)) {
    return [];
  }

  return rawRows
    .filter((row) => row && typeof row === 'object')
    .map((row) => ({
      chain_key: String(row.chain_key ?? 'unknown'),
      chain_id: Number.isFinite(row.chain_id) ? Number(row.chain_id) : null,
      source_id: String(row.source_id ?? '-'),
      state: String(row.state ?? 'unknown'),
      endpoint_index: Number.isFinite(row.endpoint_index) ? Number(row.endpoint_index) : 0,
      endpoint_count: Number.isFinite(row.endpoint_count) ? Number(row.endpoint_count) : 0,
      ws_url: String(row.ws_url ?? ''),
      http_url: String(row.http_url ?? ''),
      last_pending_unix_ms: Number.isFinite(row.last_pending_unix_ms)
        ? Number(row.last_pending_unix_ms)
        : null,
      silent_for_ms: Number.isFinite(row.silent_for_ms) ? Number(row.silent_for_ms) : null,
      updated_unix_ms: Number.isFinite(row.updated_unix_ms) ? Number(row.updated_unix_ms) : null,
      last_error: row.last_error ? String(row.last_error) : '',
      rotation_count: Number.isFinite(row.rotation_count) ? Number(row.rotation_count) : 0,
    }))
    .sort((left, right) => left.chain_key.localeCompare(right.chain_key));
}

export function formatChainStatusChainKey(chainKey) {
  const normalized = String(chainKey ?? '').trim();
  if (!normalized) {
    return 'Unknown';
  }

  const first = normalized.split('-')[0] ?? normalized;
  return first.charAt(0).toUpperCase() + first.slice(1);
}

export function formatDurationToken(durationMs) {
  if (!Number.isFinite(durationMs) || durationMs < 0) {
    return '0s';
  }

  const totalSeconds = Math.floor(durationMs / 1000);
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }

  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return `${totalMinutes}m`;
  }

  const totalHours = Math.floor(totalMinutes / 60);
  return `${totalHours}h`;
}

export function chainStatusTone(state) {
  const normalized = String(state ?? '').toLowerCase();
  if (normalized === 'active') {
    return 'bg-[#e7f1e8] text-[#244d30] border-[#2e5f3a]';
  }
  if (normalized === 'connecting' || normalized === 'subscribed' || normalized === 'booting') {
    return 'bg-[#fff2dc] text-[#6d4c18] border-[#8a621f]';
  }
  if (normalized === 'rotating') {
    return 'bg-[#ebe5ff] text-[#3f2f72] border-[#5a4aa2]';
  }
  if (normalized === 'silent_timeout' || normalized === 'error') {
    return 'bg-[#fde8e8] text-[#6b2020] border-[#7a2d2d]';
  }
  return 'bg-[#fffdf7] text-zinc-700 border-zinc-700';
}

export async function fetchJson(apiBase, path) {
  const response = await fetch(`${apiBase}${path}`);
  if (!response.ok) {
    throw new Error(`failed ${path}: HTTP ${response.status}`);
  }
  return response.json();
}

export function classifyRisk(feature) {
  const score = feature?.mev_score ?? 0;
  if (score >= 80) {
    return {
      label: 'High',
      accent: 'border-zinc-900 bg-zinc-900 text-[#f7f1e6]',
    };
  }
  if (score >= 45) {
    return {
      label: 'Medium',
      accent: 'border-zinc-900 text-zinc-800',
    };
  }
  return {
    label: 'Low',
    accent: 'border-zinc-700 text-zinc-700',
  };
}

export function statusForRow(feature) {
  if (!feature) {
    return 'Pending';
  }
  if (feature.mev_score >= 85) {
    return 'Flagged';
  }
  if (feature.urgency_score >= 70) {
    return 'Processing';
  }
  return 'Completed';
}

export function statusBadgeClass(status, inverted = false) {
  if (inverted) {
    return 'border-[#f7f1e6] text-[#f7f1e6]';
  }
  if (status === 'Completed') {
    return 'border-[#2e5f3a] bg-[#e7f1e8] text-[#244d30]';
  }
  if (status === 'Pending' || status === 'Processing') {
    return 'border-[#8a621f] bg-[#fff2dc] text-[#6d4c18]';
  }
  if (status === 'Flagged') {
    return 'border-[#7a2d2d] bg-[#fde8e8] text-[#6b2020]';
  }
  return 'border-zinc-900 text-zinc-800';
}

export function riskBadgeClass(level, inverted = false) {
  if (inverted) {
    return 'border-[#f7f1e6] text-[#f7f1e6]';
  }
  if (level === 'Low') {
    return 'border-[#2e5f3a] bg-[#e7f1e8] text-[#244d30]';
  }
  if (level === 'Medium') {
    return 'border-[#8a621f] bg-[#fff2dc] text-[#6d4c18]';
  }
  if (level === 'High') {
    return 'border-[#7a2d2d] bg-[#fde8e8] text-[#6b2020]';
  }
  return 'border-zinc-900 text-zinc-800';
}

export function opportunityCandidateTone(opportunity, active = false) {
  const strategy = String(opportunity?.strategy ?? '').toLowerCase();
  const category = String(opportunity?.category ?? '').toLowerCase();
  const palette = resolveOpportunityPalette(strategy, category);

  return active
    ? {
        container: palette.activeContainer,
        subtle: palette.activeSubtle,
      }
    : {
        container: palette.idleContainer,
        subtle: palette.idleSubtle,
      };
}

export function opportunityRowKey(opportunity) {
  const txHash = String(opportunity?.tx_hash ?? '');
  const strategy = String(opportunity?.strategy ?? '');
  const detectedUnixMs = Number.isFinite(opportunity?.detected_unix_ms)
    ? String(opportunity.detected_unix_ms)
    : '';
  return `${txHash}::${strategy}::${detectedUnixMs}`;
}

export function sparklinePath(values, width, height) {
  if (!values.length) {
    return '';
  }

  const max = Math.max(...values, 1);
  const step = values.length > 1 ? width / (values.length - 1) : width;

  return values
    .map((value, index) => {
      const x = index * step;
      const y = height - (value / max) * height;
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(2)} ${y.toFixed(2)}`;
    })
    .join(' ');
}

export function paginationWindow(currentPage, totalPages, maxButtons = 5) {
  if (!Number.isFinite(totalPages) || totalPages <= 0) {
    return [1];
  }
  if (totalPages <= maxButtons) {
    return Array.from({ length: totalPages }, (_, index) => index + 1);
  }

  const half = Math.floor(maxButtons / 2);
  let start = Math.max(1, currentPage - half);
  let end = Math.min(totalPages, start + maxButtons - 1);
  start = Math.max(1, end - maxButtons + 1);

  const pages = [];
  for (let page = start; page <= end; page += 1) {
    pages.push(page);
  }
  return pages;
}
