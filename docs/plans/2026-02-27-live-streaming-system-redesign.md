# Live Streaming System Redesign (Backend + Frontend)

Date: 2026-02-27  
Owner: Dashboard + Viz API + Storage  
Status: In Progress (Phase 3 cutover underway as of 2026-02-27)

## 1. Problem Statement

Current behavior under sustained runtime (minutes to hours):

- Dashboard snapshot fetch latency rises over time.
- Browser memory grows steadily to ~1.0-1.2 GB and does not recover sufficiently.
- Transaction detail fetch latency grows linearly after long runs.
- UI update cadence degrades even with expected upstream budget (~20 tx/s).

This is not a single bug. It is an architectural mismatch between:

- write-amplified backend read models, and
- polling-oriented frontend synchronization.

## 2. Root-Cause Evidence In Current Code

### Backend hot-path recomputation on every revision

- `InMemoryVizProvider::dashboard_cache_snapshot()` recomputes replay/features/opportunities whenever `read_model_revision` changes.
- `read_model_revision` increments for every upsert/append operation in storage.
- Result: at live ingest rate, snapshot-related aggregates are refreshed continuously.

Code references:

- `crates/viz-api/src/lib.rs`: dashboard cache refresh path (`dashboard_cache_snapshot`, `build_replay_points`, `build_feature_summary`, `build_feature_details`, `build_opportunities`)
- `crates/storage/src/lib.rs`: `bump_read_model_revision()` called by write upserts and append paths

### Snapshot recomputation includes replay build even when replay payload is disabled

- Replay payload in `/dashboard/snapshot` can be disabled via `replay_limit=0`, but cache refresh currently still rebuilds replay points inside provider cache refresh.
- Result: high CPU persists despite smaller response payload.

### Heavy cloning/sorting in hot paths

- Opportunities are materialized/sorted from storage views during cache rebuild.
- Feature summary/detail also iterate large side tables during rebuild.

### Frontend synchronization remains snapshot-heavy in degraded stream conditions

- UI still performs periodic snapshot sync paths and resyncs.
- When snapshot latency rises above snapshot cadence, network queueing occurs.
- Detail fetches contend with ongoing snapshot requests and appear linearly slower.

Code references:

- `apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js`

## 3. Non-Goals

- No full historical analytics in the dashboard hot path.
- No unbounded retention in browser or in-memory API state.
- No per-request full-table scans for dashboard reads.

## 4. Redesign Options

### Option A: Incremental Read Model (Recommended)

Backend maintains a dedicated incremental dashboard read model updated by the writer pipeline, not rebuilt on request.

Pros:

- Stable read latency (O(1)/O(log n) bounded operations).
- Bounded memory by policy.
- Clean separation: ingest/write model vs dashboard read model.

Cons:

- Requires explicit aggregator state and versioning.
- More moving parts than current direct-read approach.

### Option B: Keep current model, optimize aggressively

Retain current architecture and optimize data structures/caches.

Pros:

- Lower immediate refactor cost.

Cons:

- High risk of recurring regressions.
- Hard to guarantee bounded complexity under live writes.

### Option C: External stream/cache service

Move dashboard state/streaming to Redis/Kafka/Materialize-like layer.

Pros:

- Horizontal scalability.

Cons:

- Significant operational complexity and rollout risk now.

Recommendation: Option A now, Option C later if scale requires.

## 5. Target Architecture (Option A)

### 5.1 Backend

Introduce `DashboardReadModel` actor/state:

- Input: write events from single-writer pipeline.
- Output:
  - `snapshot_v2` precomputed payload slices (no full recompute).
  - `stream_v2` delta frames (insert/update/delete) with monotonic revision and seq watermark.

State (bounded):

- `tx_ring`: latest N transaction summaries (ring buffer).
- `tx_detail_index`: hash -> compact detail struct, LRU/TTL bounded.
- `feature_top`: bounded aggregates per protocol/category.
- `feature_recent_ring`: latest M feature details.
- `opportunity_recent_ring`: latest K opportunities.
- `chain_status`: latest per-chain status.
- `watermarks`: latest global seq and per-stream seq.

Critical policy:

- Do not compute replay in dashboard read path.
- Replay/history served by dedicated historical API only.
- Avoid storing full `raw_tx` payload in hot memory when not needed by dashboard (store optional pointer/compressed/offload).

### 5.2 API contract changes

Add new routes (keep old for transitional compatibility):

- `GET /dashboard/snapshot-v2?cursor=<optional>`
  - Returns compact initial state and `revision`.
- `GET /dashboard/stream-v2` (WebSocket)
  - Pushes delta batches:
    - `tx_upsert[]`, `tx_remove[]`
    - `feature_update[]`
    - `opp_upsert[]`, `opp_remove[]`
    - `watermark`
  - Includes backpressure/credit and gap signaling.
- `GET /transactions/{hash}` remains, served from bounded `tx_detail_index`.

### 5.3 Frontend

Move from periodic full-snapshot synchronization to:

1. Bootstrap once with `snapshot-v2`.
2. Consume `stream-v2` deltas continuously.
3. Use targeted mini-resync only on explicit gap (not periodic full sync).

Client state design:

- Normalized entity stores (`Map`) with bounded id rings per panel.
- Render lists derived from id arrays; no full-array rebuilds each tick.
- Strict caps:
  - tx rows cap
  - feature rows cap
  - opportunity rows cap
  - detail cache cap + TTL
- Main-thread render cadence decoupled from ingest cadence via worker reducer.

## 6. Backpressure + Memory Budget Rules

### Backend

- Global memory budget for dashboard read model (hard cap).
- On pressure:
  - evict oldest non-critical entities first,
  - keep latest watermark and latest tx window.
- Emit metrics:
  - `dashboard_snapshot_build_ms_p50/p95/p99`
  - `dashboard_stream_backlog`
  - `dashboard_read_model_bytes`
  - `tx_detail_index_size`
  - `dashboard_evictions_total`

### Frontend

- Single in-flight low-priority sync request allowed.
- High-priority detail request preempts low-priority sync.
- Cap worker queue size; collapse/coalesce deltas when UI lags.
- Emit browser metrics:
  - heap usage trend
  - dropped-frame ratio
  - render commit duration
  - delta queue depth

## 7. Migration Plan

### Phase 0: Instrumentation and guardrails (1-2 days)

- Add explicit metrics for:
  - cache refresh frequency,
  - snapshot rebuild duration,
  - read model memory estimates.
- Add client telemetry for queued/pending snapshot/detail requests.

### Phase 1: Backend read model split (3-5 days)

- Introduce `DashboardReadModel` updated incrementally on writes.
- Implement `snapshot-v2` backed only by read model.
- Keep existing routes for compatibility.

### Phase 2: Stream v2 and frontend reducer (3-5 days)

- Add `stream-v2` delta protocol.
- Frontend worker applies deltas to bounded normalized state.
- Disable steady full snapshot polling in normal operation.

### Phase 3: Cutover and cleanup (2-3 days)

- Switch dashboard to v2 endpoints by feature flag.
- Verify SLOs and memory ceilings.
- Decommission old snapshot-heavy path once stable.

Phase 3 execution status (updated 2026-02-27):

- Completed: frontend cutover to `/dashboard/snapshot-v2` + `/dashboard/stream-v2` only (removed v1 runtime toggles and v1 transport fallback).
- Completed: backend cutover decommissioned legacy `/dashboard/snapshot` and `/stream` routes; tests now enforce `404` on legacy endpoints.
- Completed: performance baseline test switched to `snapshot-v2` payload path.
- Remaining: 60-minute soak validation against Section 8 SLO targets (snapshot-v2 p95/p99, UI heap steady state, detail-modal latency trend).

## 8. Acceptance Criteria (SLOs)

Under sustained 20 tx/s for 60 minutes:

- Backend:
  - `snapshot-v2` p95 < 120 ms, p99 < 250 ms
  - `/transactions/{hash}` p95 < 80 ms, p99 < 150 ms
  - bounded read-model memory (no monotonic growth beyond configured cap)
- Frontend:
  - renderer heap stabilizes under configured cap (example target: < 350 MB steady-state)
  - transaction list update latency p95 < 300 ms from server delta receipt
  - no linear growth of detail modal open latency over runtime

## 9. Immediate Execution Recommendation

Start with Phase 0 + Phase 1 first. This removes the largest architectural bottleneck without requiring immediate UI rewrite, then Phase 2 can complete the full end-to-end stability fix.

## 10. Mempulse V2 Protocol Spec (Derived From Binance + Discord + Grafana Patterns)

This section turns reference patterns into concrete mempulse contracts:

- Binance pattern: snapshot + ordered deltas + strict gap recovery.
- Discord pattern: session lifecycle, heartbeat, resume.
- Grafana pattern: single socket with multiplexed channel subscriptions.

### 10.1 Transport Topology

- One WebSocket connection per browser tab to `/dashboard/stream-v2`.
- Multiplex channels over that single socket.
- Channel examples:
  - `tx.main`
  - `feature.main`
  - `opportunity.main`
  - `chain.main`
  - `market.main`

### 10.2 Session Lifecycle

Server -> client:

- `HELLO`: includes heartbeat interval and server session metadata.
- `DISPATCH`: data messages (snapshot ack, delta batches, control notices).
- `HEARTBEAT_ACK`: confirms liveness.
- `RECONNECT`: asks client to reconnect and resume.
- `INVALID_SESSION`: resume token invalid, must rebootstrap.

Client -> server:

- `IDENTIFY`: first connect with auth and desired subscriptions.
- `RESUME`: reconnect with prior `session_id` + `last_seq`.
- `HEARTBEAT`: periodic liveness.
- `SUBSCRIBE` / `UNSUBSCRIBE`: channel management.
- `CREDIT`: explicit receive budget for backpressure.

### 10.3 Bootstrap and Delta Flow

Step 1: HTTP bootstrap

- `GET /dashboard/snapshot-v2?channels=tx.main,feature.main,opportunity.main,chain.main,market.main`
- Response includes:
  - `revision`
  - `snapshot_seq`
  - bounded initial state arrays/maps
  - `resume_token_ttl_ms`

Step 2: stream start

- Open WS and send `IDENTIFY` with `snapshot_seq`.
- Server streams only events with `seq > snapshot_seq`.

Step 3: ongoing deltas

- All deltas carry monotonic `seq`.
- Client applies only in-order deltas.
- If `seq` gap detected, client requests `RESYNC` for impacted channel(s).

### 10.4 Message Shapes (Canonical)

```json
{
  "op": "HELLO",
  "heartbeat_interval_ms": 15000,
  "session_id": "sess_abc123",
  "server_time_unix_ms": 1700000000000
}
```

```json
{
  "op": "DISPATCH",
  "type": "DELTA_BATCH",
  "seq": 812345,
  "channel": "tx.main",
  "patch": {
    "upsert": [{ "hash": "0x...", "seen_unix_ms": 1700000000123 }],
    "remove": ["0x..."]
  },
  "watermark": { "latest_ingest_seq": 812345 }
}
```

```json
{
  "op": "CREDIT",
  "channel_credit": {
    "tx.main": 2,
    "feature.main": 1,
    "opportunity.main": 1
  }
}
```

```json
{
  "op": "RESUME",
  "session_id": "sess_abc123",
  "last_seq": 812345,
  "subscriptions": ["tx.main", "feature.main", "opportunity.main"]
}
```

### 10.5 Ordering and Gap Rules

- `seq` is globally monotonic for stream ordering.
- Per-channel patches are idempotent and last-write-wins by `(entity_id, seq)`.
- Gap detection rule:
  - if `incoming.seq != local.last_seq + 1`, mark gap.
  - pause apply for affected channel.
  - fetch channel-specific mini-snapshot:
    - `GET /dashboard/snapshot-v2?channels=tx.main&since_seq=<last_good_seq>`
  - resume streaming from returned `snapshot_seq`.

### 10.6 Backpressure Contract

- Server must not push data for a channel with zero credit.
- Client periodically replenishes credit using observed render throughput.
- Server coalesces bursts:
  - merge repeated updates for same entity in current flush window.
  - favor newest value per entity.
- Hard limits:
  - max entities per batch per channel.
  - max bytes per frame.
  - max flush interval.

### 10.7 Memory Bounds (Protocol-Enforced)

Server-side:

- Channel-specific retention windows/caps in `DashboardReadModel`.
- Delta batches contain only bounded slices.
- Evictions emitted as explicit `remove` patches.

Client-side:

- Bounded normalized store per channel.
- LRU+TTL for detail cache.
- On pressure:
  - drop non-critical historical rows first.
  - preserve currently selected transaction/detail.

### 10.8 Reliability and Recovery

- Heartbeat timeout -> reconnect with `RESUME`.
- Resume failure -> `INVALID_SESSION` -> full bootstrap.
- Reconnect jittered exponential backoff.
- Replay endpoint remains historical/debug only, not required for dashboard liveness.

### 10.9 Observability Requirements

Server metrics:

- `stream_v2_dispatch_latency_ms`
- `stream_v2_gap_events_total`
- `stream_v2_resume_success_total`
- `stream_v2_credit_exhausted_total`
- `snapshot_v2_build_ms`

Client metrics:

- `ui_delta_apply_ms`
- `ui_stream_gap_count`
- `ui_render_drop_ratio`
- `ui_heap_used_mb`
- `ui_detail_open_latency_ms`

## 11. Implementation Addendum (Concrete Work Breakdown)

### 11.1 Backend tasks

- Add `DashboardReadModel` state object and incremental update hooks in writer path.
- Add `/dashboard/snapshot-v2` and `/dashboard/stream-v2`.
- Add stream op-code parser/serializer (`HELLO`, `IDENTIFY`, `RESUME`, `CREDIT`, `HEARTBEAT`).
- Implement per-channel credit accounting and coalescing.
- Implement resume buffer (bounded recent seq window) and fallback invalidation.

### 11.2 Frontend tasks

- Add stream-v2 worker reducer with normalized bounded maps and id arrays.
- Replace steady snapshot polling with:
  - one bootstrap call,
  - channel mini-resync on gap only.
- Add heartbeat + resume manager in worker.
- Implement adaptive credit replenishment from measured commit throughput.

### 11.3 Rollout strategy

- Feature flags:
  - `VITE_UI_STREAM_V2_ENABLED`
  - `VIZ_API_STREAM_V2_ENABLED`
- Shadow mode first:
  - run v1 and v2 in parallel,
  - compare seq drift and entity counts.
- Flip read path after SLO stability across 60-minute soak tests.
