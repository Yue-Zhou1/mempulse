//! External state-provider abstractions for RPC-backed simulation.

use crate::Result;
use auto_impl::auto_impl;
use common::Address;

/// Minimal account state needed to seed the simulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountSeed {
    pub balance_wei: u128,
    pub nonce: u64,
}

/// Source of account state for RPC-backed simulation mode.
#[auto_impl(&, Box, Arc)]
pub trait StateProvider: Send + Sync {
    /// Returns the account seed for an address, or `None` when no external
    /// state is available.
    fn account_seed(&self, address: Address) -> Result<Option<AccountSeed>>;
}

/// State provider that always reports no external state.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopStateProvider;

impl StateProvider for NoopStateProvider {
    fn account_seed(&self, _address: Address) -> Result<Option<AccountSeed>> {
        Ok(None)
    }
}
