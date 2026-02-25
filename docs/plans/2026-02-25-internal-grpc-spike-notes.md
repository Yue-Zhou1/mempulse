# Internal gRPC Spike Notes (Task 12)

Date: 2026-02-25
Owner: backend/platform
Status: documentation spike complete

## Goal

Evaluate whether replacing the in-process ingest/write boundary with an internal gRPC service
would improve throughput, tail latency, or isolation enough to justify the added complexity.

## Current Boundary

- Ingest path writes `StorageWriteOp` directly to an in-process bounded channel.
- Single-writer task applies global sequencing and WAL append.
- Read/API path stays in-process (`InMemoryStorage` + `viz-api` providers).

This avoids serialization overhead and keeps failure domains local to one process.

## Candidate gRPC Interface (if needed later)

Potential unary/stream contract:

- `rpc AppendEvents(stream NormalizedEventEnvelope) returns (AppendAck)`
- Envelope fields:
  - `seq_hint` (optional)
  - `source_id`
  - `ingest_ts_unix_ms`
  - `event_type`
  - `payload_bytes` (normalized tx envelope)

This would require protobuf schema, codegen, service lifecycle, and retry semantics.

## Evaluation Summary

### Throughput/Latency Expectations

- In-process channel:
  - no wire encoding/decoding
  - no socket hop
  - shared-memory handoff
- Internal gRPC:
  - protobuf encode + decode cost
  - additional scheduling/context switches
  - new backpressure and retry surface

Inference: for current single-node topology, gRPC is unlikely to beat in-process handoff on P99.

### Operational Tradeoffs

Pros:
- stronger process isolation
- clearer service boundary for future multi-process split

Cons:
- new failure modes (transport, retries, partial delivery)
- duplicated queue/backpressure logic across process boundaries
- higher deploy/observability complexity

## Decision

Do not migrate ingest->storage handoff to gRPC now.

Keep in-process write path as the default and revisit only if one of these triggers occurs:

1. storage writer must run as an independently scaled service,
2. multi-process/node deployment becomes mandatory,
3. profiling shows in-process channel contention cannot be resolved with local tuning.

## Follow-up (deferred)

- If triggers are hit, open a scoped implementation spike with:
  - `proto/viz_ingest.proto`,
  - `crates/viz-api/src/grpc.rs`,
  - parity tests comparing gRPC payload semantics vs in-process `StorageWriteOp`.
