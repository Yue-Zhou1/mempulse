use event_log::TxDecoded;
use searcher::{SearcherConfig, SearcherInputTx, rank_opportunities};

fn decoded_with_calldata_len(calldata_len: usize) -> TxDecoded {
    TxDecoded {
        hash: [0x42; 32],
        tx_type: 2,
        sender: [0x11; 20],
        nonce: 7,
        chain_id: Some(1),
        to: Some([
            0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac,
            0xb4, 0xc6, 0x59, 0xf2, 0x48, 0x8d,
        ]),
        value_wei: Some(1_000_000_000_000_000),
        gas_limit: Some(320_000),
        gas_price_wei: None,
        max_fee_per_gas_wei: Some(45_000_000_000),
        max_priority_fee_per_gas_wei: Some(7_000_000_000),
        max_fee_per_blob_gas_wei: None,
        calldata_len: Some(calldata_len as u32),
    }
}

#[test]
fn opportunity_zero_copy_accepts_borrowed_calldata_without_clone() {
    let calldata = vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4];
    let expected_ptr = calldata.as_ptr() as usize;
    let batch = vec![SearcherInputTx {
        decoded: decoded_with_calldata_len(calldata.len()),
        calldata: calldata.into(),
    }];

    let ranked = rank_opportunities(
        &batch,
        SearcherConfig {
            min_score: 0,
            max_candidates: 8,
        },
    );

    let borrowed_ptr = batch[0].calldata.as_ptr() as usize;
    assert_eq!(borrowed_ptr, expected_ptr);
    assert!(!ranked.is_empty());
}
