# Backend Refactor Final Acceptance

Date: March 9, 2026

## Acceptance Summary

The backend refactor is accepted against the gap-closure plan. The architecture now matches the intended ownership split:

- scheduler owns live state transitions,
- runtime-core owns adapters and worker execution,
- storage derives read models from lifecycle events,
- replay reconstructs the full lifecycle contract,
- perf and CI gates cover the critical paths.

## Capability Checklist

### Capability A: scheduler admission is authoritative

Status: PASS

Evidence:

- `scheduler` owns admission, replacement handling, candidate registration, simulation generations, and builder handoff outputs.
- `runtime-core` routes candidate registration and simulation result application through scheduler APIs.
- Focused coverage exists in `crates/scheduler/tests/candidate_pipeline.rs` and `crates/runtime-core/src/live_rpc.rs`.

### Capability B: nonce-aware readiness is deterministic

Status: PASS

Evidence:

- scheduler snapshots preserve ready and blocked sender queues,
- replay reconstructs sender queue state deterministically from lifecycle events,
- existing scheduler and replay tests remain green under the cutover.

### Capability C: candidate generation is bounded and scheduler-owned

Status: PASS

Evidence:

- candidate state lives in scheduler snapshots and generations,
- stale candidate generations are invalidated on replacement,
- runtime-core no longer acts as the authority for candidate progression.

### Capability D: simulation gating is explicit and stale-safe

Status: PASS

Evidence:

- `CandidateQueued`, `SimDispatched`, and `SimCompleted` are explicit lifecycle artifacts,
- stale simulation results increment scheduler stale-drop metrics and do not reach builder handoff,
- runtime-core replacement tests verify stale-result rejection.

### Capability E: builder decisions are replayable and inspectable

Status: PASS

Evidence:

- `AssemblyDecisionApplied` is emitted on builder decisions,
- storage derives a builder lifecycle projection from appended events,
- replay reconstructs assembly status per candidate.

### Capability F: restart, replay, metrics, and rollback docs are complete

Status: PASS

Evidence:

- scheduler snapshots and storage rehydration plans remain in place for restart,
- replay and storage lifecycle docs are updated in this batch,
- perf regression artifacts and CI upload steps document the critical-path checks.

## Verification

The following commands passed during acceptance:

```bash
cargo test -p event-log -p replay -p storage -p runtime-core --no-fail-fast
cargo test -p bench --no-fail-fast
cargo bench -p bench --no-run
bash scripts/perf_regression_check.sh
bash scripts/verify_phase_checks.sh
cargo test --workspace
```

## Conclusion

The backend refactor should now be treated as complete. Follow-up work should target optimization, operational hardening, or future simulation backend improvements rather than ownership-model cleanup.
