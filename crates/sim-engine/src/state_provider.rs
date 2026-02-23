use anyhow::Result;
use common::Address;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountSeed {
    pub balance_wei: u128,
    pub nonce: u64,
}

pub trait StateProvider: Send + Sync {
    fn account_seed(&self, address: Address) -> Result<Option<AccountSeed>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopStateProvider;

impl StateProvider for NoopStateProvider {
    fn account_seed(&self, _address: Address) -> Result<Option<AccountSeed>> {
        Ok(None)
    }
}
