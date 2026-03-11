//! Ingest pipelines for turning live mempool inputs into canonical event-log records.

#![forbid(unsafe_code)]

use common::TxHash;
use std::error::Error as StdError;
use std::sync::Arc;

pub mod devp2p_runtime;
pub mod p2p;
pub mod rpc;
pub mod tx_decode;

type SharedError = Arc<dyn StdError + Send + Sync>;

/// Errors returned by ingest services when they talk to upstream providers or
/// normalize provider responses into internal events.
#[derive(Clone, Debug, thiserror::Error)]
pub enum IngestError {
    /// The upstream pending-hash source could not be reached or responded with
    /// an RPC-layer failure.
    #[error("RPC connection failed: {0}")]
    RpcConnect(SharedError),
    /// A transaction-specific fetch failed after a hash had already been
    /// accepted for processing.
    #[error("transaction fetch failed for {hash}: {source}")]
    TxFetch { hash: String, source: SharedError },
    /// Any other ingest error that does not need a more specific label.
    #[error(transparent)]
    Other(SharedError),
}

impl IngestError {
    pub(crate) fn into_rpc_connect(self) -> Self {
        match self {
            Self::RpcConnect(_) => self,
            other => Self::RpcConnect(Arc::new(other)),
        }
    }

    pub(crate) fn into_tx_fetch(self, hash: TxHash) -> Self {
        match self {
            Self::TxFetch { .. } => self,
            other => Self::TxFetch {
                hash: format_tx_hash(hash),
                source: Arc::new(other),
            },
        }
    }
}

impl From<anyhow::Error> for IngestError {
    fn from(error: anyhow::Error) -> Self {
        let boxed: Box<dyn StdError + Send + Sync> = error.into();
        Self::Other(Arc::from(boxed))
    }
}

fn format_tx_hash(hash: TxHash) -> String {
    let mut out = String::from("0x");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub use devp2p_runtime::*;
pub use p2p::*;
pub use rpc::*;
pub use tx_decode::*;
