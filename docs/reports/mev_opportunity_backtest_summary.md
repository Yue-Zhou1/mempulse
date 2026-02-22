# MEV Opportunity Backtest Summary

## Scope

This report summarizes deterministic candidate ranking quality from the `searcher` crate using a labeled synthetic mempool batch.

## Reproduce

```bash
bash scripts/generate_artifacts.sh
cat artifacts/searcher/backtest_summary.json
```

## Config

- `min_score=8000`
- `max_candidates=64`
- Dataset: `synthetic-mainnet-like-mix-v1` (`n=12`)

## Metrics

- Ground truth positives: `6`
- Predicted positives: `5`
- `TP=5`, `FP=0`, `FN=1`, `TN=6`
- Precision: `1.0000`
- Recall: `0.8333`
- F1: `0.9091`
- PnL proxy: `+460 bps`

## Top Ranked Signals

Top-ranked strategies were swap-heavy candidates on:

- `1inch`
- `uniswap-v2`
- `uniswap-v3`

Detailed ranking output is recorded in `artifacts/searcher/backtest_summary.json`.

## Notes

- This artifact is intended for demo demonstration and deterministic regression checks.
- For production-like evaluation, replace the synthetic labels with captured event windows and realized-on-chain outcomes.
