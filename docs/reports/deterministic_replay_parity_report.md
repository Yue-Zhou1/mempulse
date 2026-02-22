# Deterministic Replay Parity Report

## Scope

This report validates replay determinism parity at lifecycle checkpoints using the current `replay` crate logic (`lifecycle_parity`).

## Reproduce

```bash
bash scripts/generate_artifacts.sh
cat artifacts/replay/parity_report.json
```

## Result Snapshot

- Dataset: `synthetic-mixed-lifecycle-v1`
- Total events: `1911`
- Total checkpoints: `1911`
- Passed checkpoints: `1911`
- Failed checkpoints: `0`
- Parity: `100.0000%`
- SLO target: `99.99%`
- SLO status: `PASS`

## Notes

- Dataset includes mixed lifecycle transitions (`TxDecoded`, `TxConfirmedFinal`, `TxDropped`, `TxReplaced`, `TxReorged`).
- The generated artifact is at `artifacts/replay/parity_report.json`.
