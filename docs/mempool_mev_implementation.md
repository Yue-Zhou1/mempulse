# Implementation Plan - Rust High-Frequency Mempool & MEV Infrastructure

> Status: Legacy baseline plan. Active implementation roadmap is:
> - `docs/plans/2026-02-21-mev-infra-v2-implementation.md`
> - `docs/plans/v2_scope_kpi.md`

## 1. Objective

Design and implement a high-throughput, low-latency Rust system for ingesting, decoding, and analyzing Ethereum transaction supply-chain data (mempool / propagation / ordering signals) to support MEV research and block-building infrastructure.

The system will:

- Observe public transaction propagation at network speed
- Normalize mempool visibility across peers
- Extract MEV-relevant transaction features
- Provide deterministic, replayable datasets for simulation and block construction research
- Track lifecycle safely across chain reorgs
- Support hybrid visualization: operational dashboards and interactive 2D research views

---

## 2. Scope

### In-scope (Phase 1-3)
- Public mempool ingestion (Ethereum L1)
- Transaction propagation tracking
- High-frequency Rust pipeline (network -> decode -> features -> storage)
- Canonical append-only mempool event stream
- Deterministic replay mode (event-replay)
- Reorg-aware transaction lifecycle tracking
- MEV-relevant classification features
- Observability (latency, coverage, throughput)
- Hybrid visualization layer (Grafana for ops, custom 2D frontend for research)

### Out-of-scope (later phases)
- MEV strategy execution
- Bundle submission
- Block building
- Private orderflow integration
- Relay / builder infrastructure

---

## 3. Engineering Tech Details (Stack, Data Structures, Latency)

### 3.1 Stack
- reth networking crates
- tokio
- bytes
- crossbeam
- hashbrown
- jemalloc/mimalloc
- ClickHouse (primary historical/event store)
- optional PostgreSQL/Timescale sidecar (only for mutable OLTP-style workflows if needed)
- tracing + flamegraph
- Grafana OSS (operations dashboards/alerting)
- custom frontend (React + Magic UI-style 2D layout) for interactive research visualization

### 3.2 Data structures
- sharded hashmaps
- nonce chains (BTree)
- binary heap (priority scheduling)
- smallvec peer sets
- zero-copy Bytes
- append-only event log indexed by `seq_id`

### 3.3 Latency techniques
- zero-copy buffers
- batch writes
- lock-free channels
- sharded state
- cache-aligned structs
- allocator tuning
- flamegraph-driven optimization

### 3.4 Clock and timestamp model
- Capture both wall-clock (`unix_ms`) and monotonic (`mono_ns`) timestamps at ingest boundary.
- Use monotonic deltas for in-process latency measurements.
- Use NTP/PTP sync and monitor clock-skew drift for cross-host comparability.
- Record timestamp source and collector host for all latency-sensitive events.

---

## 4. System Architecture Overview

```
P2P Peers / RPC Nodes
        ↓
Mempool Ingestion Layer
        ↓
Transaction Decode + Normalization
        ↓
Canonical Event Log (append-only, ordered)
        ↓
State + Feature Derivation
        ↓
Storage + Replay + Analysis
```

---

## 5. Data Sources

### 5.1 Public Mempool (Ethereum L1)

Two ingestion modes:

#### Mode A - RPC Pending Stream (MVP)
- `eth_subscribe(newPendingTransactions)`
- `eth_getTransactionByHash`

Advantages:
- Fast to implement
- Reliable baseline

Limitations:
- Incomplete visibility
- RPC latency and batching constraints

#### Mode B - P2P Gossip (High-Frequency Target)
Implement Ethereum devp2p ETH protocol ingestion:
- `NewPooledTransactionHashes`
- `GetPooledTransactions`
- `PooledTransactions`
- `Transactions`

Goals:
- Multiple peer connectivity
- Earlier transaction arrival vs RPC
- Coverage expansion
- Propagation latency measurement

Note:
- Phase 1 can start with Mode A, but Phase 2 should treat Mode B as primary for high-fidelity propagation analysis.

---

## 6. Core Components

### 6.0 Canonical Event Log and Data Contract
All lifecycle, replay, and feature outputs are derived from a single immutable event stream.

Event envelope:
```
EventEnvelope {
  seq_id: u64,                 // globally monotonic within a collector instance
  ingest_ts_unix_ms: i64,      // wall-clock timestamp
  ingest_ts_mono_ns: u64,      // monotonic timestamp
  source_id: SourceId,         // peer/rpc/provider identity
  payload: EventPayload
}
```

Event payload types (minimum):
- `TxSeen`
- `TxFetched`
- `TxDecoded`
- `TxReplaced`
- `TxDropped`
- `TxConfirmedProvisional`
- `TxConfirmedFinal`
- `TxReorged`

Determinism rule:
- Replay applies events strictly by `seq_id` then deterministic tie-breaker (`source_id`, `hash`) if needed.

### 6.1 Mempool Ingestion Service
Responsibilities:
- Maintain peer connections and subscriptions
- Receive transaction hash announcements
- Request full transactions (body fetch)
- Deduplicate across peers
- Timestamp arrival at the earliest observation point (both monotonic + wall clock)

Event:
```
TxSeen {
  hash: B256,
  peer_id: PeerId,
  seen_at_unix_ms: i64,
  seen_at_mono_ns: u64
}
```

### 6.2 Transaction Decoder
Responsibilities:
- Decode EIP-2718 typed transactions:
  - legacy
  - EIP-2930
  - EIP-1559
  - EIP-4844 blob transactions
- Extract sender, nonce, gas fields
- Extract calldata and selector (first 4 bytes)
- Normalize fee fields with type-aware semantics

Event:
```
TxFull {
  hash: B256,
  tx_type: u8,
  chain_id: u64,
  sender: Address,
  nonce: u64,
  to: Option<Address>,
  value: U256,
  gas_limit: u64,
  gas_price: Option<u128>,
  max_fee_per_gas: Option<u128>,
  max_priority_fee_per_gas: Option<u128>,
  max_fee_per_blob_gas: Option<u128>,
  calldata: Bytes
}
```

### 6.3 Mempool State Manager
Responsibilities:
- Maintain sender nonce chains
- Detect replacements (same sender + nonce)
- Track pending lifetime and evictions
- Track confirmation status with canonical-head awareness
- Handle reorg rollback events

Events:
```
TxReplaced
TxDropped
TxConfirmedProvisional
TxConfirmedFinal
TxReorged
```

Reorg policy:
- Phase 2 minimum: detect canonical-head divergence and emit rollback/reopen lifecycle events.
- Phase 3 full: deterministic rollback and re-application with replay parity guarantees.

### 6.4 Feature Extraction Engine
Goal: produce MEV-relevant signals without execution (fast heuristics and metadata).

Features:
- contract selector match
- known protocol address mapping (DEX routers, pools, lending markets)
- calldata size / input entropy proxy
- gas percentile / fee anomalies
- replacement frequency (spam vs real intent)
- peer propagation delay (early/late arrival signals)
- MEV relevance heuristic score

Output:
```
TxFeatures {
  hash: B256,
  protocol: Option<Protocol>,
  category: TxCategory,
  gas_percentile: u16,
  propagation_ms: u32,
  mev_score: u16
}
```

### 6.5 Storage Layer
Two tiers:

#### Prometheus (real-time metrics)
- tx/sec, bytes/sec
- peer coverage
- latency (p50/p90/p99)
- decode errors
- replacements/sec
- mempool size estimate
- clock skew and timestamp drift
- queue saturation and dropped-event counters

#### Historical store
Primary decision:
- ClickHouse as default historical/event analytics store.

Optional:
- PostgreSQL/Timescale sidecar only if a strongly relational/mutable workflow is required.

Tables (suggested):
- `event_log` (seq_id, ingest_ts_unix_ms, ingest_ts_mono_ns, source_id, event_type, hash, payload)
- `tx_seen` (hash, peer, first_seen_unix_ms, first_seen_mono_ns, seen_count, min/max seen times)
- `tx_full` (raw tx bytes and normalized fields)
- `tx_features` (derived features)
- `tx_lifecycle` (replaced/dropped/confirmed/reorged timestamps, reason codes)
- `peer_stats` (per-peer throughput, drop rate, RTT estimates)

### 6.6 Replay Engine
Goal: deterministic mempool reconstruction for research and simulation input.

Replay modes:
- `deterministic_event_replay` (authoritative mode; required for research datasets)
- `snapshot_replay` (fast approximate mode for lightweight analytics)

Capabilities:
- Reconstruct pending set by time window or `seq_id` window
- Rebuild sender nonce chains and replacement history
- Reconstruct lifecycle through reorgs
- Replay per-peer arrival ordering (optional)
- Export replay frames to Parquet/CSV for external tools

Frame:
```
ReplayFrame {
  seq_hi: u64,
  timestamp_unix_ms: i64,
  pending: Vec<TxHash>
}
```

### 6.7 Visualization and Query API
Goal: serve both operational and research personas without duplicating monitoring systems.

Model:
- Grafana remains the system of record for production operations, alerting, and on-call views.
- A custom UI provides exploratory analytics and 3D interaction over replay and propagation data.

Required backend surface:
- `viz-api` read service exposing:
  - HTTP queries for historical windows (`/replay`, `/propagation`, `/features`)
  - WebSocket stream for live updates (`/stream`)
  - server-side downsampling/aggregation for rendering stability

Initial 3D views:
- peer propagation graph (nodes = peers/sources, edges = propagation delay distributions)
- time-scrubbable replay scene (pending-set evolution)
- protocol/category clustering over time

---

## 7. Performance Requirements

Target throughput (initial):
- >= 50k tx/min sustained on mainnet-like conditions
- >= 10 peers connected
- sub-millisecond ingest -> enqueue for hash announcements on warmed path
- < 50 us median decode time per tx on modern CPUs (tune by workload)

Hot-path constraints:
- minimize allocations per tx (ideally constant or amortized)
- avoid shared mutable state in ingest/decode stages
- bounded queues with explicit backpressure policy to survive spam bursts

Backpressure policy (required):
- Prioritize unique tx bodies over duplicate hash announcements.
- Drop low-value duplicate announcements first, but emit drop-reason counters/events.
- Spill to disk only in controlled mode with explicit watermark alerts.

---

## 8. Multi-Chain Considerations

Mempool scope is per network. Architect the pipeline with chain adapters:

```
ChainAdapter {
  chain_id,
  network_config,
  ingest_mode
}
```

Adapters:
- `EthereumAdapter` (public mempool via devp2p and/or RPC)
- `RollupAdapter` (sequencer feed / blocks; public mempool may be unavailable)
- `RPCAdapter` (fallback-only or restricted environments)

---

## 9. Observability

Metrics:
- `peer_count`, `peer_disconnects_total`
- `tx_hash_announcements_total`
- `tx_full_received_total`
- `tx_decode_fail_total`
- `tx_first_seen_latency_ms`
- `duplicate_rate` and `seen_count` distribution
- `replacements_total`, `drops_total`, `confirmations_total`, `reorgs_total`
- `queue_depth_*` and `backpressure_drop_total`
- `clock_skew_ms` and collector timestamp drift

Alerts:
- peer churn spike
- ingest lag / queue saturation
- decode failure rate spike
- coverage collapse (tx/sec drops vs baseline)
- storage write latency spike
- clock skew threshold breach

---

## 10. Visualization Strategy (Hybrid)

Operational UI (keep):
- Grafana for p50/p90/p99 latency, ingest health, queue depth, decode failures, and alert state.
- Alerting remains in Prometheus/Alertmanager + Grafana workflows.

Research UI (add):
- custom frontend for interaction-heavy workflows that Grafana is not optimized for
- 2D panels for topology/propagation/replay tasks where focused interaction adds signal

Boundaries:
- Do not re-implement alerting, incident dashboards, or SRE workflows in custom UI in Phase 1-3.
- Custom UI is read-only over curated APIs; no direct DB access from browser.

Rollout:
- Stage A: keep Grafana-only ops dashboards (Phase 1 baseline)
- Stage B: add `viz-api` and custom 2D replay/propagation panels
- Stage C: expand 2D interaction panels after data contracts and performance budgets are stable

---

## 11. Implementation Phases

### Phase 1 - RPC Mempool MVP
- Implement WS subscription for pending hashes
- Fetch full transactions via RPC
- Decode + feature extraction
- Write canonical `event_log` with deterministic ordering
- Store + dashboards + alerts
- Ship Grafana baseline dashboards for ops metrics

Deliverable:
- working mempool monitor with replayable dataset (RPC baseline)

Acceptance criteria:
- 24h stable run without data-loss crashes
- deterministic replay produces identical pending-set snapshots for same `seq_id` range
- decode coverage includes legacy + 2930 + 1559 + 4844

### Phase 2 - P2P High-Frequency Ingestion (+ minimum reorg safety)
- Peer manager + discovery + session handling
- ETH gossip message handling (hash announcements + body fetch)
- Dedup + earliest timestamping across peers (mono + wall clocks)
- Propagation metrics (per-peer and aggregate)
- Canonical head tracking + provisional confirmation rollback on reorg
- Introduce `viz-api` endpoints and schema contracts for replay/propagation data

Deliverable:
- high-coverage mempool tap suitable for latency/propagation research

Acceptance criteria:
- >= 10 stable peers over 24h sampling
- p99 ingest -> enqueue latency stays within defined SLO under burst test
- reorg simulation test emits rollback events and reopens affected tx lifecycle state correctly

### Phase 3 - Deterministic Replay Dataset & Full Lifecycle Tracking
- Full replace/dropped/confirmed/reorged state machine
- Reconstruct pending sets for arbitrary windows
- Export replay frames (Parquet)
- Add deterministic replay CLI keyed by time/seq range
- Add snapshot replay mode as non-authoritative fast path
- Implement custom frontend with initial 2D views (propagation graph + replay timeline)

Deliverable:
- simulation-ready mempool history with reorg-aware lifecycle metadata

Acceptance criteria:
- replay determinism test: repeated runs over same event window produce identical outputs
- lifecycle parity test: online state vs replayed state matches at checkpoint boundaries
- end-to-end export/import test validates reproducible downstream analysis inputs
- UI performance budget: interactive views maintain usable frame rate on target hardware with downsampled datasets

---

## 12. Rust Project Structure

```
mempool-rs/
  crates/
    p2p-ingest/
    rpc-ingest/
    event-log/
    tx-decode/
    mempool-state/
    feature-engine/
    storage/
    viz-api/
    replay/
    common/
  apps/
    web-ui/
```

---

## 13. System Design Diagram

```mermaid
flowchart LR

subgraph Network
    P2P[devp2p peers]
    RPC[RPC nodes]
end

subgraph Ingestion
    P2PIng[p2p-ingest]
    RPCIng[rpc-ingest]
    Decode[tx-decode]
end

subgraph Core
    Log[event-log]
    State[mempool-state]
    Feature[feature-engine]
end

subgraph Storage
    CH[(ClickHouse primary)]
    TS[(Timescale optional)]
    Metrics[(Prometheus)]
end

subgraph Analysis
    Replay[replay-engine]
end

subgraph Visualization
    API[viz-api]
    Grafana[Grafana]
    WebUI[custom-web-ui (2D Magic UI style)]
end

P2P --> P2PIng
RPC --> RPCIng
P2PIng --> Decode
RPCIng --> Decode
Decode --> Log
Log --> State
State --> Feature
Feature --> CH
Feature -. optional .-> TS
Feature --> Metrics
Log --> Replay
CH --> Replay
Metrics --> Grafana
CH --> API
Replay --> API
API --> ThreeUI
Metrics --> API
```

---

## 14. Expected Outcomes

After Phase 2-3 the system will provide:

- High-fidelity mempool visibility
- Propagation latency analysis and coverage diagnostics
- Reorg-aware transaction lifecycle tracking
- Deterministic replayable pending sets and feature-rich datasets
- Clear authoritative vs approximate replay modes
- Hybrid observability UX: stable ops dashboards plus interactive research visualization

This forms the foundational transaction-gathering layer beneath block builders and MEV research pipelines.

---

## 15. References (selected)
- reth networking docs: https://reth.rs/docs/reth_network/index.html
- hashbrown HashMap docs: https://docs.rs/hashbrown/latest/hashbrown/struct.HashMap.html
- crossbeam-channel docs: https://docs.rs/crossbeam-channel
- tracing docs: https://docs.rs/tracing
- tikv-jemallocator docs: https://docs.rs/tikv-jemallocator/
- mimalloc crate: https://crates.io/crates/mimalloc
- cargo-flamegraph: https://github.com/killercup/cargo-flamegraph
- ClickHouse insert strategy: https://clickhouse.com/docs/best-practices/selecting-an-insert-strategy
- ClickHouse transactional behavior: https://clickhouse.com/docs/guides/developer/transactional
- Timescale hypertables: https://docs.timescale.com/use-timescale/latest/hypertables/
- flashbots mempool-dumpster: https://github.com/flashbots/mempool-dumpster
- Grafana docs: https://grafana.com/docs/
- Magic UI docs: https://magicui.design/docs
