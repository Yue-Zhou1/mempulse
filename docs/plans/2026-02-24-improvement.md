# Mempulse Comprehensive Improvement Plan (reth-Style Engineering Roadmap)

> Goal: Upgrade mempulse from a "runnable MEV pipeline prototype" to an engineering system that is **replayable (deterministic)**, **rollback-capable (reorg-aware)**, **observable (tracing/metrics)**, and **extensible (pluggable lanes/strategies/relays/providers)**.
>
> Covered pipeline: Ingest -> Canonical Event Log -> Replay/Lifecycle -> Feature/Ranking -> Simulation/Builder Dry-run -> Delivery (API/UI/Alerts).

---

## 0. Non-Negotiable Engineering Principles

1. **Single Source of Truth: Canonical Event Log**
    - Every fact must first be written as **append-only typed events**; all downstream state is derived (replayable).
2. **Determinism First**
    - Any ordering that affects derived computation must be defined by a **computable key** (not thread scheduling, not HashMap iteration order).
3. **Backpressure Is a Feature**
    - All ingest queues must be **bounded**; when full, drop and write a `Dropped{reason}` event (for postmortem replay).
4. **Reorg Does Not Rewrite History, Only Appends Events**
    - Reorg is handled via `ReorgDetected`, which drives replay and emits derived events such as reopen/invalidated.
5. **Observability Is Part of the Interface**
    - Full-chain correlation must be possible: `opp_id -> sim_id -> bundle_id -> relay_attempt -> outcome`.

---

## 1. Target Architecture (reth-ish crate layering)

Recommended workspace layout (close to your current structure, with harder boundaries and better replaceability):

```
mempulse/
crates/
mempulse-primitives/      # Hash/Id/PeerId/NormalizedTx/Block + event schema (no IO)
mempulse-canon/           # global sequencer + WAL + broadcast bus + scan/read API
mempulse-net/             # RPC lane + devp2p lane (I/O boundary: decode/light validation/enqueue)
mempulse-replay/          # deterministic replay engine + checkpoints + reorg handling
mempulse-features/        # classification + feature extraction + scoring (versioned rules)
mempulse-sim/             # deterministic revm sim + trace capture
mempulse-builder/         # relay adapters + retry/backoff + outcomes
mempulse-storage/         # clickhouse sink + read model indexes (query acceleration)
mempulse-api/             # HTTP/WS API (events/replay/features/sim/bundles)
mempulse-observability/   # tracing/metrics helpers + prom rules templates
bin/
mempulse-node/            # wiring: lanes -> sequencer -> consumers -> api
```

---

## 2. Workstream A: Determinism Scaffolding (Highest Priority)

### A1. Introduce a Global Sequencer (unified `global_seq`)

**Problem**: If multiple lanes (RPC + P2P) emit concurrently and each increments its own seq, merged order becomes dependent on receive timing (non-deterministic).

**Solution**: Lanes emit only `LocalEvent`; the Sequencer assigns `global_seq`, then broadcasts only after WAL append succeeds.

**Recommended data structures (example)**

```rust
// primitives
pub type EventId = u64;

#[derive(Clone, Copy, Debug)]
pub enum LaneId { Rpc, P2p }

#[derive(Clone, Debug)]
pub struct LocalEvent {
  pub lane: LaneId,
  pub local_seq: u64,
  pub ts_ns: u64,          // observation timestamp, analysis only, not part of ordering
  pub source_id: SourceId, // peer/rpc-provider label
  pub keys: EventKeys,     // tx/block/opp/peer (for indexing)
  pub kind: EventKind,     // typed payload
}

#[derive(Clone, Debug)]
pub struct CanonicalEvent {
  pub id: EventId,         // global_seq: unique ordering root
  pub ts_ns: u64,
  pub source_id: SourceId,
  pub keys: EventKeys,
  pub kind: EventKind,
}
```

**Acceptance**

- For the same input (fixed seed/recorded replay), repeated runs produce identical `id` sequence and identical final `state_digest`.

---

### A2. Prevent HashMap Iteration Order from Leaking into Visible Output

**Mandatory actions**

- Any collection output entering these paths must be sorted:
    - event writes (batch order)
    - checkpoint hash / snapshot serialization
    - API responses (lists, summaries)
    - Prometheus label output (when needed)
- Rule: `collect -> sort_unstable_by_key -> emit`

**Suggested CI gate**

- `determinism_replay_test`: replay the same event segment twice; `checkpoint_hash` computed every N events must match (>=99.99% parity).

---

### A3. Rule Versioning (feature/scorer/strategy)

- `FeatureEngine::version()` / `Scorer::version()` / `Strategy::version()` must be written into related events (`OppDetected/SimCompleted/BundleSubmitted`).
- Replay must depend only on: event stream + version numbers (eliminate implicit global config drift).

---

## 3. Workstream B: Ingest Engineering (RPC + devp2p), Backpressure, Propagation Timing

### B1. Unified Queue Topology (bounded + drop event)

**Recommended topology**

```
[lane tasks] -> (bounded) -> [dedup/normalize workers] -> (append canonical events)
                         -> drop -> TxDropped/BlockDropped(reason, queue, depth)
```

**Standardized drop reasons**

- `QueueFull | DecodeFail | Oversize | Duplicate | PeerThrottled | InvalidProtocol`

Every drop must include:

- `lane_id, source_id, queue_name, depth_current, depth_peak, reason`

---

### B2. Decouple devp2p lane: runtime vs ingest service

Goal: Like reth-network, isolate "protocol runtime" from "business ingest":

- runtime: peer/session/handshake/capabilities/request-response plumbing
- ingest: gossip trigger fetch, parsing, dedup, backpressure, event emission

**Interface sketch**

```rust
#[async_trait::async_trait]
pub trait P2pRuntime {
    async fn subscribe(&self) -> tokio::sync::mpsc::Receiver<P2pMsg>;
    async fn request_pooled_txs(&self, peer: PeerId, hashes: Vec<TxHash>) -> anyhow::Result<()>;
    async fn request_block_parts(&self, peer: PeerId, req: BlockReq) -> anyhow::Result<()>;
}
```

---

### B3. Propagation Timing as a First-Class Signal

Record consistently (event fields or derivable fields):

- `arrival_ns` (per observation)
- `first_seen_ns` (global earliest)
- `first_seen_by_source` (RPC vs P2P)
- `peer_first_seen_ns`

Derived metrics:

- `inclusion_latency = included_at - first_seen`
- `peer_lead_time` (average lead by peer)
- `rebroadcast_count` (heat)

These feed the urgency component in Feature/Ranking.

---

## 4. Workstream C: Canonical Event Log (WAL + ClickHouse sink + schema evolution)

### C1. WAL as the only source of truth (crash recovery)

**Recommended WAL format**

- segment files (e.g., 64MB / 256MB)
- framing: `len(u32) + payload + crc(u32)`
- writes: batch append (fewer syscalls)
- reads: `scan(from_id, limit)` (shared by API and replay)

---

### C2. ClickHouse as analysis/query acceleration layer (not the only write path)

Suggested wide table:

- `id, ts_ns, kind, source, tx, block, opp, peer, payload_json`
- `ORDER BY (id)` (most common scan)

Add MVs (aggregated by kind) when necessary.

---

### C3. Schema evolution rules

- New fields: Optional, backward compatible
- Breaking changes: introduce new event types to replace old ones; do not reuse old field semantics
- Payload format: prefer JSON (debug-friendly) or msgpack (space-saving); pick one and keep it fixed

---

## 5. Workstream D: Replay/Lifecycle Engine (checkpoint, reorg-aware mempool)

### D1. Replay engine: subscribe to canonical bus, apply strictly by id order

- Single-thread apply (guarantees determinism)
- Derived outputs (`TxStateChanged`, `OppInvalidated`, etc.) must also go through the sequencer (unified ordering)

---

### D2. Mempool state model (recommended)

- `tx_hash -> TxEntry` (lifecycle, first_seen, best_source, included_in)
- `sender -> NonceChain` (gap tracking + replacement history)
- `confirmed set` / `pending set` / `dropped set` (derivable)

NonceChain recommended implementation:

- `BTreeMap<nonce, TxHash>` + extra tracking:
    - contiguous ready nonce head
    - future gap set
    - replacement history (multiple txs for same nonce)

---

### D3. Checkpointing

- Every N events (or every T seconds), create checkpoint:
    - `state_digest` (hash)
    - `snapshot` (compressible, used for fast startup)
- Replay startup: load latest checkpoint + WAL scan

---

### D4. Reorg handling (do not rewrite history)

- Input: `ReorgDetected{old_head,new_head,common_ancestor,depth}`
- Actions:
    - reopen included txs (`confirmed -> pending`)
    - invalidate opps dependent on old head (emit events)
- Key rule: rollback logic must depend only on event stream, not wall-clock time

---

## 6. Workstream E: Feature + Opportunity Ranking (classification, heuristics, strategy)

### E1. Classification (protocol/category)

- registry supports hot updates (JSON/CH table)
- output features must be structured and explainable:
    - `protocol, selector, token_path, amounts, deadline, fee_fields`
    - `risk_flags`: replaceable / nonce_gap / unknown_to / high_slippage, etc.

### E2. Decompose scoring (explainable)

- `urgency`: TTL, propagation heat, competition intensity, reorg risk
- `value_est`: fast upper bound (without full sim)
- `risk`: historical failure rate, relay filter risk, state sensitivity

### E3. Stable sorting

Candidate output must be stable:

- `sort key = (-score_total, -urgency, tx_hash)`, etc.

---

## 7. Workstream F: Simulation + Builder Dry-run (deterministic revm + relay)

### F1. Fix deterministic revm environment

- anchor: `base_block_hash`
- env: timestamp/basefee/coinbase/spec_id only from base block
- input: raw tx/bundle bytes (`Bytes`), output: structured outcome

```rust
pub enum SimOutcome {
    Ok { profit_wei: i128, gas_used: u64, logs_digest: B256 },
    Fail { category: SimFailCategory, trace_id: u64 },
}
```

### F2. Trace capture (debug lifeline)

- failure categories: revert / out-of-gas / nonce mismatch / state mismatch
- store trace digest; persist full trace by sampling policy

### F3. Relay idempotency + retry/backoff

- `bundle_id = hash(bundle_bytes || target_block || strategy_version)`
- attempt events must record: relay, attempt_no, backoff_ms, error_class
- maintain relay health, support downweighting/circuit breaker

---

## 8. Workstream G: Delivery (API / UI / Alerts)

### G1. API (read model + WAL scan)

Recommended endpoints:

- `GET /events?after=&types=` (monotonic output by id)
- `GET /replay?from=&to=` (returns checkpoint hash and diff summary)
- `GET /tx/{hash}` (status + features + lifecycle)
- `GET /opps?status=&min_score=`
- `GET /sim/{id}` / `GET /bundle/{id}` (full-chain linkage)

### G2. UI (three highest-ROI screens)

1. Mempool radar (`first_seen`, propagation heat, peer lead)
2. Opp console (`detected -> sim -> submitted -> outcome`)
3. Replay debugger (pinpoint by id and state diff)

### G3. Prometheus metrics and alerts

Metrics:

- ingest: `queue_depth`, `drops_total{reason}`, `decode_fail_total`
- replay: `replay_lag_events`, `checkpoint_duration`, `reorg_depth`
- sim: `sim_latency`, `sim_fail_total{category}`
- relay: `relay_success_rate`, `bundle_included_total`, `bundle_filtered_total{reason}`

Alerts:

- continuously growing replay lag
- drop spikes (DoS or upstream anomalies)
- abnormal reorg depth
- relay success rate below threshold

---

## 9. Performance and Concurrency Strategy (tokio/bytes/crossbeam/hashbrown/jemalloc)

- I/O boundary: `tokio::mpsc::channel(cap)` (bounded)
- CPU worker pool: `crossbeam_channel::bounded(cap)` (low overhead)
- broadcast: `tokio::broadcast`
- zero-copy: `Bytes` through ingest -> feature -> sim (avoid `Vec` clones)
- hot-path maps: `hashbrown` + sharded locks (avoid giant `RwLock`)
- counters: `AtomicU64/AtomicUsize` (fewer locks)
- allocator: jemalloc feature flag (enabled by default for bench/profile)

---

## 10. Testing and Validation (turn replayability into a CI gate)

1. **Determinism Replay Test (core)**
    - fixed input event stream (recorded WAL segment)
    - replay twice: `checkpoint_hash` every N events must match
2. **Property/Fuzz**
    - RLP decode fuzz (oversize/depth/edge)
    - replacement/nonce/reorg sequence fuzz, verify invariants (index consistency, valid state machine transitions)
3. **Integration**
    - RPC+P2P concurrent ingest: compare `first_seen` distribution and `inclusion_latency`
    - sim -> relay: end-to-end events must be fully linked
4. **Benchmark**
    - ingest throughput and drop curves
    - replay events/s and checkpoint cost
    - sim p50/p95 and failure category distribution
    - relay success/filter reason distribution

---

## 11. Phased Milestones (Executable Task Checklist)

### Phase 0: Determinism Scaffolding (must do first)

- [x]  Global Sequencer (`global_seq + WAL append + broadcast`)
- [x]  Sort all visible collection outputs
- [x]  Write rule versions into events
- [x]  Determinism replay test (checkpoint parity)

**Acceptance**

- Repeated reruns produce identical summaries; checkpoint parity >= 99.99%

---

### Phase 1: Ingest Engineering + WAL/CH

- [x]  devp2p runtime trait abstraction/integration (or integrate with reth-network)
- [x]  All queues bounded + drop events
- [x]  WAL segments + scan API
- [x]  ClickHouse sink batch writes (acceleration only)

**Acceptance**

- WAL fully recoverable after crash; drop reasons are explainable

---

### Phase 2: Replay/Feature/Sim/Relay Enhancements

- [x]  checkpoint/snapshot
- [x]  Reorg invalidation/reopen event closed loop
- [x]  Ranking componentization + stable sorting
- [x]  Sim trace capture + improved failure categorization
- [x]  Relay health + circuit breaker/downweighting

**Acceptance**

- Reorg scenarios replay consistently; `opp -> bundle -> outcome` is traceable end-to-end

---

### Phase 3: Delivery (API/UI/Alerts)

- [x]  Full API (`events/replay/tx/opp/sim/bundle`)
- [x]  Three-screen UI
- [x]  Prometheus metrics + alert rules

**Acceptance**

- Any opp can be traced from UI to outcome; replay debugger can locate root cause

---

## 12. Low-Churn Refactor Recommendations (code movement strategy)

- Split event-log: `primitives` (schema/types) + `canon` (sequencer/WAL/bus)
- Split ingest: `net/lane/{rpc,p2p}.rs` + `net/runtime.rs`
- Add replay modules: `checkpoint.rs`, `reorg.rs`
- Merge feature-engine/searcher: `features` (extract/score/strategy)
- `sim-engine` -> `sim` (provider trait + deterministic env)
- `builder` -> `builder` (relay adapters + retry)
- `viz-api` -> `api` (read model + WAL scan)

### 12.1 Suggested directory structure (low-churn rearrangement)

- Split `event-log`:
    - `mempulse-primitives`: types + event schema
    - `mempulse-canon`: sequencer + WAL + bus + reader
- `ingest` -> `mempulse-net`:
    - `src/lane/rpc.rs`
    - `src/lane/p2p.rs`
    - `src/runtime.rs` (devp2p runtime trait)
- Add to `replay`:
    - `src/checkpoint.rs`
    - `src/reorg.rs`
- Merge `feature-engine + searcher` into:
    - `mempulse-features` (extract/score/strategy)
- `sim-engine` -> `mempulse-sim` (provider trait + deterministic env)
- `builder` -> `mempulse-builder` (relay adapters + retry)
- `viz-api` -> `mempulse-api` (read model)

### 12.2 Core trait sketches (reth-like replaceable components)

```rust
// mempulse-net
#[async_trait::async_trait]
pub trait IngestLane: Send {
    async fn run(self, out: crossbeam_channel::Sender<LocalEvent>) -> anyhow::Result<()>;
}

// mempulse-canon
#[async_trait::async_trait]
pub trait EventWriter: Send + Sync {
    async fn append_batch(&self, batch: Vec<EventEnvelope>) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait EventReader: Send + Sync {
    async fn scan(&self, from: u64, limit: usize) -> anyhow::Result<Vec<EventEnvelope>>;
}

// mempulse-replay
pub trait ReplayEngine {
    fn apply(&mut self, ev: &EventEnvelope);
    fn state_digest(&self) -> [u8; 32];
}

// mempulse-features
pub trait FeatureExtractor {
    fn version(&self) -> &'static str;
    fn extract(&self, tx: &TxDecoded) -> TxFeatures;
}

pub trait Scorer {
    fn version(&self) -> &'static str;
    fn score(&self, f: &TxFeatures) -> Score;
}

// mempulse-sim
#[async_trait::async_trait]
pub trait StateProvider: Send + Sync {
    async fn state_at(&self, base: B256) -> anyhow::Result<Arc<dyn EvmState>>;
    async fn env_at(&self, base: B256) -> anyhow::Result<EvmEnv>;
}

pub trait Simulator {
    fn simulate(&self, env: &EvmEnv, bundle: &[Bytes]) -> SimOutcome;
}

// mempulse-builder
#[async_trait::async_trait]
pub trait RelayClient: Send + Sync {
    async fn submit_bundle(&self, b: Bundle) -> anyhow::Result<SubmitAck>;
    async fn query_outcome(&self, id: BundleId) -> anyhow::Result<BundleOutcome>;
}
```

---

## 13. Minimum Slice You Can Open PRs for Immediately (start with these three)

1. **Sequencer + global_seq** (change only event write path, no business logic changes)
2. Explicit sorting for all exported/summary/API list outputs (eliminate HashMap-order leakage)
3. Determinism replay test (recorded WAL segment, CI gate)
