use common::{Address, TxHash};
use event_log::TxDecoded;
use searcher::{SearcherConfig, SearcherInputTx, StrategyKind, rank_opportunity_batch};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
}

fn tx(
    hash_v: u8,
    to: Address,
    gas_limit: u64,
    priority_fee_wei: u128,
    calldata_len: u32,
) -> TxDecoded {
    TxDecoded {
        hash: hash(hash_v),
        tx_type: 2,
        sender: address(hash_v),
        nonce: hash_v as u64,
        chain_id: Some(1),
        to: Some(to),
        value_wei: Some(1_000_000_000_000_000),
        gas_limit: Some(gas_limit),
        gas_price_wei: None,
        max_fee_per_gas_wei: Some(45_000_000_000),
        max_priority_fee_per_gas_wei: Some(priority_fee_wei),
        max_fee_per_blob_gas_wei: None,
        calldata_len: Some(calldata_len),
    }
}

#[test]
fn opportunity_scoring_is_deterministic_and_prunes() {
    let uniswap_v2 = [
        0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
        0xc6, 0x59, 0xf2, 0x48, 0x8d,
    ];
    let unknown_dex = address(0x44);
    let erc20 = address(0xee);

    let batch = vec![
        SearcherInputTx::owned(
            tx(0x10, uniswap_v2, 320_000, 7_000_000_000, 256),
            vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4, 5],
        ),
        SearcherInputTx::owned(
            tx(0x11, uniswap_v2, 290_000, 6_000_000_000, 192),
            vec![0x04, 0xe4, 0x5a, 0xaf, 9, 8, 7, 6, 5],
        ),
        SearcherInputTx::owned(
            tx(0x20, unknown_dex, 240_000, 3_000_000_000, 160),
            vec![0x50, 0x23, 0xb4, 0xdf, 1, 1, 1, 1],
        ),
        SearcherInputTx::owned(
            tx(0x30, erc20, 65_000, 1_000_000_000, 68),
            vec![0xa9, 0x05, 0x9c, 0xbb, 0, 0, 0, 0],
        ),
    ];

    let config = SearcherConfig {
        min_score: 8_000,
        max_candidates: 3,
    };
    let ranked_a = rank_opportunity_batch(&batch, config).candidates;
    let ranked_b = rank_opportunity_batch(&batch, config).candidates;

    assert_eq!(ranked_a, ranked_b);
    assert_eq!(ranked_a.len(), 3);
    assert!(
        ranked_a
            .iter()
            .all(|candidate| candidate.score >= config.min_score)
    );
    assert!(
        ranked_a
            .windows(2)
            .all(|window| window[0].score >= window[1].score)
    );
    assert_eq!(ranked_a[0].strategy, StrategyKind::SandwichCandidate);
    assert_eq!(ranked_a[0].tx_hash, hash(0x10));
}

#[test]
fn contiguous_same_sender_swaps_produce_a_bundle_candidate() {
    let uniswap_v2 = [
        0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
        0xc6, 0x59, 0xf2, 0x48, 0x8d,
    ];
    let sender = address(0xaa);
    let batch = vec![
        SearcherInputTx::owned(
            TxDecoded {
                hash: hash(0x40),
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
                hash: hash(0x41),
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

    let ranked = rank_opportunity_batch(
        &batch,
        SearcherConfig {
            min_score: 0,
            max_candidates: 8,
        },
    )
    .candidates;

    assert!(ranked.iter().any(|candidate| {
        format!("{:?}", candidate.strategy) == "BundleCandidate"
            && candidate
                .reasons
                .iter()
                .any(|reason| reason.contains("bundle"))
    }));
}
