use common::{Address, TxHash};
use event_log::TxDecoded;
use sim_engine::{ChainContext, simulate_deterministic};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
}

#[test]
fn deterministic_replay_same_inputs_produce_identical_outputs() {
    let chain = ChainContext {
        chain_id: 1,
        block_number: 19_100_000,
        block_timestamp: 1_710_000_000,
        gas_limit: 30_000_000,
        base_fee_wei: 35_000_000_000,
        coinbase: address(0x42),
        state_root: hash(0x7f),
    };

    let txs = vec![
        TxDecoded {
            hash: hash(0x01),
            tx_type: 2,
            sender: address(0xaa),
            nonce: 1,
            chain_id: Some(1),
            to: Some(address(0xbb)),
            value_wei: Some(1_000_000_000_000),
            gas_limit: Some(80_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(45_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(64),
        },
        TxDecoded {
            hash: hash(0x02),
            tx_type: 2,
            sender: address(0xab),
            nonce: 2,
            chain_id: Some(1),
            to: Some(address(0xbc)),
            value_wei: Some(0),
            gas_limit: Some(95_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(52_000_000_000),
            max_priority_fee_per_gas_wei: Some(3_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(256),
        },
    ];

    let run_a = simulate_deterministic(&chain, &txs).expect("run a should simulate");
    let run_b = simulate_deterministic(&chain, &txs).expect("run b should simulate");

    assert_eq!(run_a.tx_results.len(), txs.len());
    assert_eq!(run_a, run_b);
    for (result_a, result_b) in run_a.tx_results.iter().zip(run_b.tx_results.iter()) {
        assert_eq!(result_a.gas_used, result_b.gas_used);
        assert_eq!(result_a.success, result_b.success);
        assert_eq!(result_a.state_diff_hash, result_b.state_diff_hash);
    }
}
