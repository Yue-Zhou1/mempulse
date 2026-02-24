use common::Address;
use event_log::TxDecoded;
use sim_engine::{
    AccountSeed, ChainContext, SimulationFailCategory, SimulationMode, SimulationTxInput,
    StateProvider, simulate_with_mode,
};

fn address(v: u8) -> Address {
    [v; 20]
}

fn hash(v: u8) -> [u8; 32] {
    [v; 32]
}

fn context() -> ChainContext {
    ChainContext {
        chain_id: 1,
        block_number: 20_000_000,
        block_timestamp: 1_710_000_000,
        gas_limit: 30_000_000,
        base_fee_wei: 1_000_000_000,
        coinbase: address(0x55),
        state_root: hash(0xaa),
    }
}

struct FixedNonceProvider {
    nonce: u64,
}

impl StateProvider for FixedNonceProvider {
    fn account_seed(&self, _address: Address) -> anyhow::Result<Option<AccountSeed>> {
        Ok(Some(AccountSeed {
            balance_wei: 2_000_000_000_000_000_000_000_000_000_000,
            nonce: self.nonce,
        }))
    }
}

#[test]
fn rpc_backed_nonce_mismatch_is_categorized_and_trace_id_is_present() {
    let provider = FixedNonceProvider { nonce: 0 };
    let txs = vec![SimulationTxInput {
        decoded: TxDecoded {
            hash: hash(1),
            tx_type: 2,
            sender: address(0x11),
            nonce: 7,
            chain_id: Some(1),
            to: Some(address(0x22)),
            value_wei: Some(0),
            gas_limit: Some(120_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(30_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(4),
        },
        calldata: Some(vec![0xde, 0xad, 0xbe, 0xef]),
    }];

    let batch = simulate_with_mode(&context(), &txs, SimulationMode::RpcBacked(&provider))
        .expect("simulation batch");

    assert_eq!(batch.tx_results.len(), 1);
    let result = &batch.tx_results[0];
    assert!(!result.success);
    assert_eq!(
        result.fail_category,
        Some(SimulationFailCategory::NonceMismatch)
    );
    assert_ne!(result.trace_id, 0);
}

#[test]
fn create_loop_out_of_gas_is_categorized() {
    // init code: JUMPDEST; PUSH1 0x00; JUMP -> loops until gas exhausts
    let loop_init_code = vec![0x5b, 0x60, 0x00, 0x56];
    let txs = vec![SimulationTxInput {
        decoded: TxDecoded {
            hash: hash(2),
            tx_type: 2,
            sender: address(0x44),
            nonce: 0,
            chain_id: Some(1),
            to: None,
            value_wei: Some(0),
            gas_limit: Some(40_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(30_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(loop_init_code.len() as u32),
        },
        calldata: Some(loop_init_code),
    }];

    let batch = simulate_with_mode(&context(), &txs, SimulationMode::SyntheticDeterministic)
        .expect("simulation batch");
    let result = &batch.tx_results[0];
    assert!(!result.success);
    assert_eq!(result.fail_category, Some(SimulationFailCategory::OutOfGas));
    assert_ne!(result.trace_id, 0);
}
