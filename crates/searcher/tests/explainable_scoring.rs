use common::{Address, TxHash};
use event_log::TxDecoded;
use searcher::{
    SearcherConfig, SearcherInputTx, StrategyKind, rank_opportunity_batch, scorer_version,
    strategy_version,
};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
}

fn tx(hash_v: u8, to: Address) -> TxDecoded {
    TxDecoded {
        hash: hash(hash_v),
        tx_type: 2,
        sender: address(hash_v),
        nonce: hash_v as u64,
        chain_id: Some(1),
        to: Some(to),
        value_wei: Some(1_000_000_000_000_000),
        gas_limit: Some(320_000),
        gas_price_wei: None,
        max_fee_per_gas_wei: Some(45_000_000_000),
        max_priority_fee_per_gas_wei: Some(7_000_000_000),
        max_fee_per_blob_gas_wei: None,
        calldata_len: Some(256),
    }
}

#[test]
fn ranked_candidates_include_score_breakdown_and_reasons() {
    let uniswap_v2 = [
        0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
        0xc6, 0x59, 0xf2, 0x48, 0x8d,
    ];
    let batch = vec![SearcherInputTx::owned(
        tx(0x44, uniswap_v2),
        vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4, 5, 6, 7, 8],
    )];

    let ranked = rank_opportunity_batch(
        &batch,
        SearcherConfig {
            min_score: 0,
            max_candidates: 8,
        },
    )
    .candidates;

    let top = ranked.first().expect("at least one candidate");
    assert!(!top.reasons.is_empty());
    assert!(
        top.reasons
            .iter()
            .any(|reason| reason.contains("mev_score"))
    );
    assert_eq!(top.feature_engine_version, feature_engine::version());
    assert_eq!(top.scorer_version, scorer_version());
    assert_eq!(top.strategy_version, strategy_version(top.strategy));
    assert_eq!(
        top.score,
        top.breakdown.mev_component
            + top.breakdown.urgency_component
            + top.breakdown.structural_component
            + top.breakdown.strategy_bonus
    );
    assert_eq!(top.member_tx_hashes, vec![top.tx_hash]);
}

#[test]
fn ranked_batch_reports_bounded_metrics_and_bundle_attribution() {
    let uniswap_v2 = [
        0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
        0xc6, 0x59, 0xf2, 0x48, 0x8d,
    ];
    let sender = address(0xaa);
    let batch = vec![
        SearcherInputTx::owned(
            TxDecoded {
                hash: hash(0x70),
                tx_type: 2,
                sender,
                nonce: 7,
                chain_id: Some(1),
                to: Some(uniswap_v2),
                value_wei: Some(1_000_000_000_000_000),
                gas_limit: Some(320_000),
                gas_price_wei: None,
                max_fee_per_gas_wei: Some(45_000_000_000),
                max_priority_fee_per_gas_wei: Some(7_000_000_000),
                max_fee_per_blob_gas_wei: None,
                calldata_len: Some(256),
            },
            vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4, 5, 6, 7, 8],
        ),
        SearcherInputTx::owned(
            TxDecoded {
                hash: hash(0x71),
                tx_type: 2,
                sender,
                nonce: 8,
                chain_id: Some(1),
                to: Some(uniswap_v2),
                value_wei: Some(1_500_000_000_000_000),
                gas_limit: Some(330_000),
                gas_price_wei: None,
                max_fee_per_gas_wei: Some(46_000_000_000),
                max_priority_fee_per_gas_wei: Some(8_000_000_000),
                max_fee_per_blob_gas_wei: None,
                calldata_len: Some(264),
            },
            vec![0x38, 0xed, 0x17, 0x39, 8, 7, 6, 5, 4, 3, 2, 1],
        ),
    ];

    let result = rank_opportunity_batch(
        &batch,
        SearcherConfig {
            min_score: 0,
            max_candidates: 2,
        },
    );

    assert_eq!(result.metrics.input_transactions, 2);
    assert!(result.metrics.generated_candidates > result.candidates.len());
    assert_eq!(result.metrics.returned_candidates, result.candidates.len());
    assert!(result.metrics.truncated_candidates > 0);
    assert_eq!(result.metrics.bundle_candidates_generated, 1);
    let bundle = result
        .candidates
        .iter()
        .find(|candidate| candidate.strategy == StrategyKind::BundleCandidate)
        .expect("bundle candidate included in bounded result");
    assert_eq!(bundle.member_tx_hashes, vec![hash(0x70), hash(0x71)]);

    assert_eq!(result.candidates.len(), 2);
}
