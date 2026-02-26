# UI Performance Improvement Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Refactor `apps/web-ui` dashboard streaming/rendering so high-frequency mempool traffic stays responsive while keeping memory bounded.

**Architecture:** Move stream ingest + normalization + buffering to a dedicated Web Worker, enforce strict fixed-cap data retention, and render only viewport-visible rows with virtualization. Keep React main-thread work to atomic frame-rate commits and lightweight selectors. Preserve current UX behavior while adding observability and hard performance gates.

**Tech Stack:** React 19 + Vite, Web Workers, ring buffer/circular buffer utilities, `zustand` (or equivalent external store), `react-virtuoso` (or `react-window`), Node test runner for pure modules with browser API shims, browser performance profiling, Transferable Objects, optional Rust/Wasm decode path.

## Review Summary (Current Frontend Bottlenecks)

- `apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js`
  - Main thread handles stream socket events, snapshot fetches, JSON parsing, row stabilization, and many state updates per cycle.
  - Snapshot apply path updates multiple large arrays (`transactions`, `feature_details`, `opportunities`, `replay`, `propagation`) in one effect.
- `apps/web-ui/src/features/dashboard/hooks/use-dashboard-derived-state.js`
  - Repeated filtering/searching/selection scans over transaction/opportunity arrays every render cycle.
  - Pagination still keeps large row sets in JS and can trigger broad re-computation.
- `apps/web-ui/src/features/dashboard/components/radar-screen.jsx`
  - Table renders all rows in current page slice; no virtualization/flyweight row reuse.
  - Row memoization exists but overall DOM scale still grows with high row limits.
- `apps/web-ui/src/features/dashboard/domain/tx-live-store.js`
  - Has cap + age pruning but still runs on main thread and depends on snapshot payload size/commit cadence.

## Scope and Performance Targets

### In Scope

- Dashboard live stream path (`radar` + shared controller hooks).
- Client memory bound enforcement and eviction policy.
- Transaction list rendering strategy and DOM pressure reduction.
- React update frequency and selector granularity.
- Profiling instrumentation and regression tests.

### Out of Scope (this plan)

- Backend API schema changes beyond optional worker-friendly deltas.
- Replay/archive feature redesign.
- Visual theme redesign.

### Release Gates (must pass)

- JS heap plateaus under `200 MB` during 30-minute synthetic run (`>= 100 tx/s`).
- No sustained Long Task over `50 ms` during steady-state load.
- Dashboard interaction latency (filter input, row click) stays under `100 ms` p95.
- Render loop remains smooth (`>= 50 FPS` typical on baseline dev machine).
- No unbounded row growth; transaction buffers hard-cap at configured maximum.
- In overload mode, frame-drop guardrail engages within `2 s` and recovers automatically when frame delta normalizes.
- Emergency memory-pressure policy prevents heap from crossing `400 MB` for more than `10 s`.

## Refinements from Additional Suggestions

- Quick win priority updated: virtualization now executes immediately after baseline instrumentation.
- Test environment trap covered: add a shared browser-API shim setup for `Worker`, `requestAnimationFrame`, Transferable-friendly message stubs, and `ResizeObserver` so task tests do not fail due to missing runtime APIs.
- Memory determinism tightened: reactive state keeps only row-level fields needed for current list view; full tx detail stays in non-reactive cache (and optional indexedDB tier).
- Selection UX clarified: keep only `activeSelectedTxId` in reactive store, and resolve full detail payload lazily from non-reactive cache at detail-pane mount time.
- CSS isolation hardened: transaction row containers use `contain: layout style;` to reduce global style/layout invalidation.
- Health guardrails added: frame-drop sampling mode (`rAF delta > 30 ms`) and memory-pressure emergency purge (`usedJSHeapSize > 400 MB` in Chrome).
- Extreme-volume path added as optional phase: worker-side binary decode via Rust/Wasm and Transferable `ArrayBuffer` payloads.
- Future SAB requirement documented: if upgrading to `SharedArrayBuffer`, dev/prod hosts must emit `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`.

## Target Module Layout (Evolution Path)

Use this structure inside dashboard feature scope to avoid broad churn in one PR:

```text
apps/web-ui/src/features/dashboard/
├── core/
│   ├── worker/
│   │   └── mempool.worker.js
│   └── store/
│       └── dashboard-stream-store.js
├── components/
│   └── stream/
│       ├── radar-virtualized-table.jsx
│       └── tx-row.jsx
├── hooks/
│   └── use-dashboard-stream-worker.js
└── domain/
    └── circular-buffer.js
```

Compatibility note: keep current files as thin adapters during migration, then collapse once rollout flags are removed.

---

### Task 1: Baseline Profiling + Guardrails

**Files:**
- Create: `apps/web-ui/src/features/dashboard/lib/perf-metrics.js`
- Create: `apps/web-ui/src/features/dashboard/lib/perf-metrics.test.mjs`
- Create: `apps/web-ui/src/test/setup-browser-apis.mjs`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js`
- Modify: `apps/web-ui/package.json`
- Modify: `apps/web-ui/README.md`

**Step 0: Configure browser API test harness (prerequisite)**

Create a shared setup module and preload it in tests:
- mock `globalThis.Worker` with deterministic message channel behavior
- patch `requestAnimationFrame`/`cancelAnimationFrame` with timer-backed shims
- provide dummy `ResizeObserver` (needed by virtualization stack)
- keep Transferable-friendly `postMessage` signatures so worker protocol tests run in Node

**Step 1: Write the failing test**

Add tests for `perf-metrics` helpers:
- long-task aggregation
- rolling FPS calculator
- heap sample reducer
- dropped-frame ratio
- API-shim smoke test (verifies setup module exposes Worker/rAF/ResizeObserver in test runtime)

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL because perf metrics module is not implemented.

**Step 3: Write minimal implementation**

- Implement lightweight perf metric helpers and optional `window.__MEMPULSE_PERF__` debug surface.
- Add periodic marks around snapshot apply/render commit path.
- Document profiling runbook in `apps/web-ui/README.md`.

**Step 4: Run test to verify it passes**

Run: `npm --prefix apps/web-ui test`
Expected: PASS with new `perf-metrics` tests included in script.

**Step 5: Commit**

```bash
git add apps/web-ui/src/features/dashboard/lib/perf-metrics.js apps/web-ui/src/features/dashboard/lib/perf-metrics.test.mjs apps/web-ui/src/test/setup-browser-apis.mjs apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js apps/web-ui/package.json apps/web-ui/README.md
git commit -m "test: add ui perf baseline metrics and profiling hooks"
```

---

### Task 2: Worker Protocol + Off-Main-Thread Stream Pipeline

**Files:**
- Create: `apps/web-ui/src/features/dashboard/workers/dashboard-stream.worker.js`
- Create: `apps/web-ui/src/features/dashboard/workers/stream-protocol.js`
- Create: `apps/web-ui/src/features/dashboard/workers/stream-protocol.test.mjs`
- Create: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-stream-worker.js`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-controller.js`

**Step 1: Write the failing test**

Add protocol tests covering:
- worker init config message validation
- delta batch payload schema
- sequence gap detection flags
- worker error message normalization

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL before protocol module and tests are wired.

**Step 3: Write minimal implementation**

- Move WebSocket + JSON parse + sequence bookkeeping into worker.
- Worker keeps internal mutable buffers and emits batched updates every `100-150 ms`.
- Main thread hook (`use-dashboard-stream-worker`) consumes typed worker messages only.
- Use Transferable payloads where possible (`postMessage(..., [arrayBuffer])`) to minimize copies for larger batch payloads.
- Keep snapshot fallback path for reconnection/recovery and progressive rollout.

**Step 4: Run test to verify it passes**

Run:
- `npm --prefix apps/web-ui test`
- `npm --prefix apps/web-ui run dev` (manual smoke: stream connects, updates flow)

Expected: tests pass and live updates remain functional.

**Step 5: Commit**

```bash
git add apps/web-ui/src/features/dashboard/workers/dashboard-stream.worker.js apps/web-ui/src/features/dashboard/workers/stream-protocol.js apps/web-ui/src/features/dashboard/workers/stream-protocol.test.mjs apps/web-ui/src/features/dashboard/hooks/use-dashboard-stream-worker.js apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js apps/web-ui/src/features/dashboard/hooks/use-dashboard-controller.js
git commit -m "feat: move dashboard stream ingest and batching to web worker"
```

---

### Task 3: Hard Memory Bound with Circular Buffers

**Files:**
- Create: `apps/web-ui/src/features/dashboard/domain/circular-buffer.js`
- Create: `apps/web-ui/src/features/dashboard/domain/circular-buffer.test.mjs`
- Modify: `apps/web-ui/src/features/dashboard/domain/tx-live-store.js`
- Modify: `apps/web-ui/src/features/dashboard/domain/tx-live-store.test.mjs`
- Modify: `apps/web-ui/src/features/dashboard/domain/ui-config.js`
- Modify: `apps/web-ui/src/features/dashboard/domain/ui-config.test.mjs`
- Modify: `apps/web-ui/public/runtime-config.js`

**Step 1: Write the failing test**

Add tests for:
- fixed-cap append with deterministic eviction order
- stable reference reuse when unchanged
- age-based prune interaction with hard cap
- config bounds for new max buffer controls

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL before circular buffer and new config fields are implemented.

**Step 3: Write minimal implementation**

- Introduce explicit constants (for example `MAX_TRANSACTIONS=1000`, tunable by runtime config).
- Replace open-ended array operations with circular buffer semantics.
- Apply same cap policy to supporting collections (`features`, `opportunities`, detail cache indexes where needed).
- Normalize reactive row shape to only list-critical fields; move full transaction detail into non-reactive `Map` (optional indexedDB spill for replay/history).
- Keep a lightweight selected-id pointer only; do not place full detail payload back into reactive store.
- Ensure old references are eligible for GC immediately after eviction.

**Step 4: Run test to verify it passes**

Run: `npm --prefix apps/web-ui test`
Expected: PASS with deterministic bounded-store behavior.

**Step 5: Commit**

```bash
git add apps/web-ui/src/features/dashboard/domain/circular-buffer.js apps/web-ui/src/features/dashboard/domain/circular-buffer.test.mjs apps/web-ui/src/features/dashboard/domain/tx-live-store.js apps/web-ui/src/features/dashboard/domain/tx-live-store.test.mjs apps/web-ui/src/features/dashboard/domain/ui-config.js apps/web-ui/src/features/dashboard/domain/ui-config.test.mjs apps/web-ui/public/runtime-config.js
git commit -m "perf: enforce hard memory caps with circular buffers for dashboard stream data"
```

---

### Task 4: Virtualized Radar List Rendering

**Files:**
- Modify: `apps/web-ui/package.json`
- Create: `apps/web-ui/src/features/dashboard/components/radar-virtualized-table.jsx`
- Modify: `apps/web-ui/src/features/dashboard/components/radar-screen.jsx`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-derived-state.js`
- Modify: `apps/web-ui/src/index.css` (or dashboard stylesheet file)

**Step 1: Write the failing test**

Add focused tests for row model shaping:
- visible-window data adapter returns stable row keys/props
- selected-row mapping remains correct under virtualization offsets

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL before virtualization adapter exists.

**Step 3: Write minimal implementation**

- Add `react-virtuoso` (preferred) or `react-window`.
- Replace `<tbody>{map(...)}</tbody>` full rendering with virtualized row renderer.
- Keep sticky header behavior and click-to-select semantics.
- Add CSS containment on row container using `contain: layout style;` (`content-visibility: auto` optional where behavior is safe).

**Step 4: Run test to verify it passes**

Run:
- `npm --prefix apps/web-ui test`
- `npm --prefix apps/web-ui run dev` and profile scroll/render

Expected: visible DOM rows remain bounded (~20-50 rows), interactions still work.

**Step 5: Commit**

```bash
git add apps/web-ui/package.json apps/web-ui/src/features/dashboard/components/radar-virtualized-table.jsx apps/web-ui/src/features/dashboard/components/radar-screen.jsx apps/web-ui/src/features/dashboard/hooks/use-dashboard-derived-state.js apps/web-ui/src/index.css
git commit -m "perf: virtualize radar transaction list rendering"
```

---

### Task 5: Atomic State Store + rAF-Batched UI Commits

**Files:**
- Modify: `apps/web-ui/package.json`
- Create: `apps/web-ui/src/features/dashboard/domain/dashboard-stream-store.js`
- Create: `apps/web-ui/src/features/dashboard/domain/dashboard-stream-store.test.mjs`
- Create: `apps/web-ui/src/features/dashboard/hooks/use-selected-transaction-detail.js`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-controller.js`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-derived-state.js`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js`
- Modify: `apps/web-ui/src/features/dashboard/components/radar-screen.jsx`
- Modify: `apps/web-ui/src/features/dashboard/components/transaction-dialog.jsx`

**Step 1: Write the failing test**

Add tests for store behavior:
- multiple worker batches merged into one frame commit
- selector-only updates do not re-render unrelated slices
- stale selection fallback logic remains correct after eviction
- `activeSelectedTxId` drives detail view while full detail object remains outside reactive store

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL before external store and frame-batching exist.

**Step 3: Write minimal implementation**

- Introduce external store (`zustand`) for stream domain state.
- Worker updates go to transient buffer; main thread commits via `requestAnimationFrame`.
- Replace broad Context-like top-level state fanout with selector-driven subscriptions.
- Split selectors by field groups so single-column updates do not force unrelated cells/rows to re-render.
- Store `activeSelectedTxId` in Zustand and keep full tx details in non-reactive cache (`Map`).
- Add dedicated selector/hook that reads selected ID from Zustand and resolves full payload from non-reactive cache only when detail pane mounts/opens.
- Keep `React.memo` row optimization with strict prop equality.

**Step 4: Run test to verify it passes**

Run:
- `npm --prefix apps/web-ui test`
- `npm --prefix apps/web-ui run dev` and verify reduced render count in React Profiler

Expected: batched commits at frame cadence, fewer dashboard-wide renders.

**Step 5: Commit**

```bash
git add apps/web-ui/package.json apps/web-ui/src/features/dashboard/domain/dashboard-stream-store.js apps/web-ui/src/features/dashboard/domain/dashboard-stream-store.test.mjs apps/web-ui/src/features/dashboard/hooks/use-selected-transaction-detail.js apps/web-ui/src/features/dashboard/hooks/use-dashboard-controller.js apps/web-ui/src/features/dashboard/hooks/use-dashboard-derived-state.js apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js apps/web-ui/src/features/dashboard/components/radar-screen.jsx apps/web-ui/src/features/dashboard/components/transaction-dialog.jsx
git commit -m "perf: adopt atomic dashboard stream store with raf-batched updates"
```

---

### Task 6: Verification Suite + Rollout Controls

**Files:**
- Create: `scripts/ui_perf_stress.sh`
- Create: `artifacts/ui-perf/.gitkeep`
- Create: `apps/web-ui/src/features/dashboard/lib/system-health.js`
- Create: `apps/web-ui/src/features/dashboard/lib/system-health.test.mjs`
- Modify: `apps/web-ui/README.md`
- Modify: `apps/web-ui/src/features/dashboard/domain/ui-config.js`
- Modify: `apps/web-ui/src/features/dashboard/domain/ui-config.test.mjs`
- Modify: `apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js`

**Step 1: Write the failing test**

Add tests for rollout flags:
- worker pipeline enable/disable
- virtualization enable/disable
- target batch interval and max buffer bounds
- frame-drop detector threshold and sampling mode transition
- memory-pressure detector and emergency purge trigger
- sampling-mode trailing-buffer flush timeout (force flush after no new batches for `500 ms`)

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL before feature flags and config parsing are implemented.

**Step 3: Write minimal implementation**

- Add runtime feature flags in `window.__MEMPULSE_UI_CONFIG__` for safe canary rollout.
- Create stress script that runs synthetic stream load and captures:
  - heap samples
  - long-task counts
  - FPS summaries
- Add "System Health" monitor:
  - if `requestAnimationFrame` delta exceeds `30 ms`, enter sampling mode (render every 5th batch) until stable.
  - if `performance.memory.usedJSHeapSize` exceeds `400 MB` (Chrome), trigger emergency purge + warning state.
- Add trailing-buffer protection: if sampling mode is active and no new batches arrive for `500 ms`, call `flush()` to render remaining buffered items and avoid stale tail rows.
- Document pass/fail thresholds and rollback instructions.

**Step 4: Run test to verify it passes**

Run:
- `npm --prefix apps/web-ui test`
- `bash scripts/ui_perf_stress.sh`

Expected: tests pass; artifact generated under `artifacts/ui-perf/` with gate summary.

**Step 5: Commit**

```bash
git add scripts/ui_perf_stress.sh artifacts/ui-perf/.gitkeep apps/web-ui/src/features/dashboard/lib/system-health.js apps/web-ui/src/features/dashboard/lib/system-health.test.mjs apps/web-ui/README.md apps/web-ui/src/features/dashboard/domain/ui-config.js apps/web-ui/src/features/dashboard/domain/ui-config.test.mjs apps/web-ui/src/features/dashboard/hooks/use-dashboard-lifecycle-effects.js
git commit -m "chore: add ui perf verification gates and rollout controls"
```

---

### Task 7 (Optional): Extreme-Volume Binary Decode in Worker (Rust/Wasm)

**Files:**
- Create: `apps/web-ui/src/features/dashboard/workers/decode-worker-bridge.js`
- Create: `apps/web-ui/src/features/dashboard/workers/wasm-decode-protocol.test.mjs`
- Create: `apps/web-ui/src/features/dashboard/workers/wasm/` (module scaffold)
- Modify: `apps/web-ui/src/features/dashboard/workers/dashboard-stream.worker.js`
- Modify: `apps/web-ui/vite.config.js`
- Modify: `apps/web-ui/README.md`

**Step 1: Write the failing test**

Add tests for:
- binary payload decode contract
- Transferable `ArrayBuffer` handoff contract
- fallback to JSON parser when wasm decoder unavailable

**Step 2: Run test to verify it fails**

Run: `npm --prefix apps/web-ui test`
Expected: FAIL before bridge/protocol is added.

**Step 3: Write minimal implementation**

- Add optional worker decode adapter that routes binary payloads through wasm module.
- Pass decoded batches to main thread via Transferables.
- Keep JSON path as default and wasm path behind runtime flag.
- Document SAB prerequisites for future zero-copy ring-buffer mode:
  - dev (`vite.config.js`) and production hosts must emit `Cross-Origin-Opener-Policy: same-origin`
  - and `Cross-Origin-Embedder-Policy: require-corp`

**Step 4: Run test to verify it passes**

Run:
- `npm --prefix apps/web-ui test`
- `npm --prefix apps/web-ui run dev` with wasm flag on/off

Expected: both paths work; wasm path reduces parse overhead under synthetic extreme load.

**Step 5: Commit**

```bash
git add apps/web-ui/src/features/dashboard/workers/decode-worker-bridge.js apps/web-ui/src/features/dashboard/workers/wasm-decode-protocol.test.mjs apps/web-ui/src/features/dashboard/workers/dashboard-stream.worker.js apps/web-ui/src/features/dashboard/workers/wasm apps/web-ui/vite.config.js apps/web-ui/README.md
git commit -m "perf: add optional wasm decode path for extreme stream volumes"
```

---

## Risks and Mitigations

- Worker/main-thread protocol drift
  - Mitigation: strict versioned message schema + protocol tests.
- Virtualized table behavior regressions (selection, keyboard/mouse interactions)
  - Mitigation: adapter tests + manual QA checklist on selection/pagination/search.
- Over-throttling updates causing stale UI
  - Mitigation: keep emergency immediate flush on critical events (errors, reconnect, explicit user actions).
- Store migration complexity
  - Mitigation: feature-flagged incremental cutover, keep old path for one release cycle.

## Operational Runbook (Post-Implementation)

- Default runtime settings:
  - `txHistoryLimit`: `1000`
  - `txRenderLimit`: `100`
  - `streamBatchMs`: `120`
  - `workerEnabled`: `true`
  - `virtualizedTickerEnabled`: `true`
  - `samplingLagThresholdMs`: `30`
  - `samplingStride`: `5`
  - `heapEmergencyPurgeMb`: `400`
- Weekly perf check:
  - run `bash scripts/ui_perf_stress.sh`
  - verify latest artifact remains inside release gates.
- If enabling future SAB mode:
  - verify COOP/COEP headers are present in local Vite responses and production CDN/reverse proxy responses before turning on the flag.

## Execution Order

1. Task 1: Baseline metrics and profiling hooks.
2. Task 4: Virtualized radar list (quick win).
3. Task 2: Worker protocol and ingest offload.
4. Task 3: Circular-buffer hard caps + normalized reactive state.
5. Task 5: Atomic store and rAF batching.
6. Task 6: Verification gates and staged rollout.
7. Task 7: Optional wasm decode phase for extreme-volume environments.

Plan complete and saved to `docs/plans/2026-02-26-ui-performance-improvement-plan.md`.
