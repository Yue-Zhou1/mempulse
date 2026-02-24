#![forbid(unsafe_code)]

mod state_provider;

use anyhow::Result;
use common::{Address, TxHash};
use event_log::TxDecoded;
use revm::context_interface::ContextTr;
use revm::database::InMemoryDB;
use revm::primitives::{Address as RevmAddress, Bytes, U256, hardfork::SpecId};
use revm::state::AccountInfo;
use revm::{Context, DatabaseCommit, ExecuteEvm, MainBuilder, MainContext, context::TxEnv};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub use state_provider::{AccountSeed, NoopStateProvider, StateProvider};

const MAX_SYNTHETIC_CALLDATA_BYTES: u32 = 8_192;
const SEEDED_BALANCE_WEI: u128 = 1_000_000_000_000_000_000_000_000_000_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChainContext {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_timestamp: u64,
    pub gas_limit: u64,
    pub base_fee_wei: u128,
    pub coinbase: Address,
    pub state_root: TxHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxSimulationResult {
    pub hash: TxHash,
    pub success: bool,
    pub gas_used: u64,
    pub state_diff_hash: TxHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationBatchResult {
    pub chain_context: ChainContext,
    pub tx_results: Vec<TxSimulationResult>,
    pub final_state_diff_hash: TxHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SimulationTxInput {
    pub decoded: TxDecoded,
    #[serde(default)]
    pub calldata: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
pub enum SimulationMode<'a> {
    SyntheticDeterministic,
    RpcBacked(&'a dyn StateProvider),
}

pub fn simulate_deterministic(
    chain_context: &ChainContext,
    txs: &[TxDecoded],
) -> Result<SimulationBatchResult> {
    let inputs = txs
        .iter()
        .cloned()
        .map(|decoded| SimulationTxInput {
            decoded,
            calldata: None,
        })
        .collect::<Vec<_>>();
    simulate_with_mode(
        chain_context,
        &inputs,
        SimulationMode::SyntheticDeterministic,
    )
}

pub fn simulate_with_mode(
    chain_context: &ChainContext,
    txs: &[SimulationTxInput],
    mode: SimulationMode<'_>,
) -> Result<SimulationBatchResult> {
    let mut db = InMemoryDB::default();
    seed_sender_accounts(&mut db, txs, mode)?;

    let base_fee_u64 = chain_context.base_fee_wei.min(u64::MAX as u128) as u64;
    let mut evm = Context::mainnet()
        .with_db(db)
        .modify_cfg_chained(|cfg| {
            cfg.chain_id = chain_context.chain_id;
            cfg.spec = SpecId::CANCUN;
            cfg.disable_nonce_check = false;
        })
        .modify_block_chained(|block| {
            block.number = chain_context.block_number;
            block.timestamp = chain_context.block_timestamp;
            block.gas_limit = chain_context.gas_limit;
            block.basefee = base_fee_u64;
            block.beneficiary = to_revm_address(chain_context.coinbase);
        })
        .build_mainnet();

    let mut aggregate = Sha256::new();
    aggregate.update(chain_context.state_root);
    aggregate.update(chain_context.chain_id.to_le_bytes());
    aggregate.update(chain_context.block_number.to_le_bytes());
    aggregate.update(chain_context.block_timestamp.to_le_bytes());

    let mut tx_results = Vec::with_capacity(txs.len());
    for tx in txs {
        let tx_env = tx_env_from_input(chain_context, tx);
        let result_and_state = evm.transact(tx_env)?;
        let gas_used = result_and_state.result.gas_used();
        let success = result_and_state.result.is_success();
        let state_diff_hash = hash_state_diff(&result_and_state.state);

        aggregate.update(tx.decoded.hash);
        aggregate.update(gas_used.to_le_bytes());
        aggregate.update([u8::from(success)]);
        aggregate.update(state_diff_hash);

        evm.db().commit(result_and_state.state);
        tx_results.push(TxSimulationResult {
            hash: tx.decoded.hash,
            success,
            gas_used,
            state_diff_hash,
        });
    }

    Ok(SimulationBatchResult {
        chain_context: chain_context.clone(),
        tx_results,
        final_state_diff_hash: aggregate.finalize().into(),
    })
}

fn seed_sender_accounts(
    db: &mut InMemoryDB,
    txs: &[SimulationTxInput],
    mode: SimulationMode<'_>,
) -> Result<()> {
    let mut sender_start_nonce = BTreeMap::<Address, u64>::new();
    for tx in txs {
        sender_start_nonce
            .entry(tx.decoded.sender)
            .and_modify(|nonce| *nonce = (*nonce).min(tx.decoded.nonce))
            .or_insert(tx.decoded.nonce);
    }

    for (sender, nonce) in sender_start_nonce {
        let seeded = match mode {
            SimulationMode::SyntheticDeterministic => None,
            SimulationMode::RpcBacked(provider) => provider.account_seed(sender)?,
        };
        let balance = seeded
            .map(|account| account.balance_wei)
            .unwrap_or(SEEDED_BALANCE_WEI);
        let account_nonce = seeded.map(|account| account.nonce).unwrap_or(nonce);
        let info = AccountInfo::from_balance(U256::from(balance)).with_nonce(account_nonce);
        db.insert_account_info(to_revm_address(sender), info);
    }
    Ok(())
}

fn tx_env_from_input(chain_context: &ChainContext, input: &SimulationTxInput) -> TxEnv {
    let tx = &input.decoded;
    let gas_limit = tx.gas_limit.unwrap_or(210_000).max(21_000);
    let mut gas_price = tx
        .gas_price_wei
        .or(tx.max_fee_per_gas_wei)
        .unwrap_or(chain_context.base_fee_wei.max(1));
    if gas_price < chain_context.base_fee_wei {
        gas_price = chain_context.base_fee_wei;
    }

    let gas_priority_fee = tx
        .max_priority_fee_per_gas_wei
        .map(|v| v.min(gas_price))
        .filter(|v| *v > 0);
    let calldata_len = tx
        .calldata_len
        .unwrap_or(0)
        .min(MAX_SYNTHETIC_CALLDATA_BYTES);
    let data = input
        .calldata
        .as_ref()
        .map(|bytes| {
            let bounded_len = bytes.len().min(MAX_SYNTHETIC_CALLDATA_BYTES as usize);
            Bytes::from(bytes[..bounded_len].to_vec())
        })
        .unwrap_or_else(|| Bytes::from(vec![0u8; calldata_len as usize]));

    TxEnv {
        tx_type: tx.tx_type,
        caller: to_revm_address(tx.sender),
        gas_limit,
        gas_price,
        kind: tx.to.map(to_revm_address).into(),
        value: U256::from(tx.value_wei.unwrap_or_default()),
        data,
        nonce: tx.nonce,
        chain_id: Some(tx.chain_id.unwrap_or(chain_context.chain_id)),
        access_list: Default::default(),
        gas_priority_fee,
        blob_hashes: Vec::new(),
        max_fee_per_blob_gas: tx.max_fee_per_blob_gas_wei.unwrap_or_default(),
        authorization_list: Vec::new(),
    }
}

fn hash_state_diff(state: &revm::state::EvmState) -> TxHash {
    let mut hasher = Sha256::new();
    let mut accounts: Vec<_> = state.iter().collect();
    accounts.sort_by(|(a, _), (b, _)| a.as_slice().cmp(b.as_slice()));

    for (address, account) in accounts {
        hasher.update(address.as_slice());
        hasher.update(account.info.balance.to_string().as_bytes());
        hasher.update(account.info.nonce.to_le_bytes());
        hasher.update(account.info.code_hash.as_slice());
        hasher.update([account.status.bits()]);

        let mut storage_slots: Vec<_> = account.changed_storage_slots().collect();
        storage_slots.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (slot, value) in storage_slots {
            hasher.update(slot.to_string().as_bytes());
            hasher.update(value.present_value().to_string().as_bytes());
        }
    }

    hasher.finalize().into()
}

fn to_revm_address(address: Address) -> RevmAddress {
    RevmAddress::from_slice(&address)
}
