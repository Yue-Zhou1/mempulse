use common::{Address, TxHash};
use event_log::TxDecoded;
use sim_engine::{AccountSeed, ChainContext, SimulationMode, SimulationTxInput, StateProvider, simulate_with_mode};
use std::sync::atomic::{AtomicUsize, Ordering};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
}

struct MockStateProvider {
    calls: AtomicUsize,
}

impl MockStateProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn account_calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl StateProvider for MockStateProvider {
    fn account_seed(&self, _address: Address) -> anyhow::Result<Option<AccountSeed>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(AccountSeed {
            balance_wei: 2_000_000_000_000_000_000_000_000_000_000,
            nonce: 0,
        }))
    }
}

#[test]
fn rpc_backed_mode_fetches_account_state_before_execution() {
    let provider = MockStateProvider::new();
    let context = ChainContext {
        chain_id: 1,
        block_number: 20_000_000,
        block_timestamp: 1_710_000_000,
        gas_limit: 30_000_000,
        base_fee_wei: 1_000_000_000,
        coinbase: address(0x55),
        state_root: hash(0xaa),
    };
    let txs = vec![SimulationTxInput {
        decoded: TxDecoded {
            hash: hash(1),
            tx_type: 2,
            sender: address(0x11),
            nonce: 0,
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

    let result = simulate_with_mode(&context, &txs, SimulationMode::RpcBacked(&provider))
        .expect("rpc-backed simulation result");
    assert_eq!(result.tx_results.len(), 1);
    assert!(provider.account_calls() > 0);
}
