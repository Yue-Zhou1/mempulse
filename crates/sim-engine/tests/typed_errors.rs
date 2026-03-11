use common::{Address, TxHash};
use event_log::TxDecoded;
use sim_engine::{
    AccountSeed, ChainContext, SimError, SimulationMode, SimulationTxInput, StateProvider,
    simulate_with_mode,
};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
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

struct FailingStateProvider;

impl StateProvider for FailingStateProvider {
    fn account_seed(&self, _address: Address) -> Result<Option<AccountSeed>, SimError> {
        Err(SimError::state_fetch(std::io::Error::other(
            "rpc unavailable",
        )))
    }
}

#[test]
fn rpc_backed_state_provider_errors_are_typed() {
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

    let err = simulate_with_mode(&context(), &txs, SimulationMode::RpcBacked(&FailingStateProvider))
        .expect_err("state fetch should fail");

    assert!(matches!(err, SimError::StateFetch(_)));
    assert!(err.to_string().contains("state fetch failed"));
    assert!(err.to_string().contains("rpc unavailable"));
}

#[test]
fn reverted_error_displays_reason() {
    let err = SimError::Reverted {
        reason: "execution reverted".to_owned(),
    };

    assert_eq!(err.to_string(), "EVM execution reverted: execution reverted");
}

#[test]
fn state_provider_auto_impl_supports_arc_wrappers() {
    fn fetch_seed<P: StateProvider>(provider: &P) -> Result<Option<AccountSeed>, SimError> {
        provider.account_seed(address(0x42))
    }

    struct FixedProvider;

    impl StateProvider for FixedProvider {
        fn account_seed(&self, _address: Address) -> Result<Option<AccountSeed>, SimError> {
            Ok(Some(AccountSeed {
                balance_wei: 42,
                nonce: 7,
            }))
        }
    }

    let provider = std::sync::Arc::new(FixedProvider);
    let seed = fetch_seed(&provider).expect("arc provider should implement trait");

    assert_eq!(
        seed,
        Some(AccountSeed {
            balance_wei: 42,
            nonce: 7,
        })
    );
}
