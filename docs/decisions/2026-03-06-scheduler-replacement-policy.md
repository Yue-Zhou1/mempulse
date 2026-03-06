# Scheduler Replacement Policy

Date: 2026-03-06
Status: Accepted

## Context

Sprint 2 of the backend refactor requires the replacement policy to be locked before nonce-aware sender queues are implemented. The scheduler needs a deterministic rule for same-sender, same-nonce conflicts and for bounding sender-local memory growth under spam.

## Decision

The scheduler uses the following replacement policy for the nonce-aware mempool:

1. Same-sender, same-nonce replacement requires a minimum fee bump of 10%.
2. The fee used for the comparison is `gas_price` for legacy/EIP-2930 transactions and `max_fee_per_gas` for EIP-1559/EIP-4844 transactions.
3. A replacement candidate that does not meet the 10% bump is rejected silently by the scheduler and counted in a dedicated metric; the incumbent transaction remains in place.
4. The maximum number of pending transactions per sender is 64. New non-replacement transactions beyond that limit are dropped and counted in a dedicated metric.
5. If two same-nonce transactions have identical fee parameters, the incumbent transaction wins. First-seen order is the tie-breaker.

## Consequences

- The scheduler remains deterministic under replay because replacement acceptance depends only on sender, nonce, and fee fields already present in the validated transaction contract.
- The initial policy is intentionally simple and stable. It can be refined later if bundle-aware or base-fee-aware replacement semantics become necessary.
- Metrics must expose underpriced replacement rejections and per-sender capacity drops so operators can see when the policy is actively filtering traffic.
