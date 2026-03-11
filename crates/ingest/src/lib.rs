#![forbid(unsafe_code)]

use common::TxHash;
use std::error::Error as StdError;
use std::sync::Arc;

pub mod devp2p_runtime;
pub mod p2p;
pub mod rpc;
pub mod tx_decode;

type SharedError = Arc<dyn StdError + Send + Sync>;

#[derive(Clone, Debug, thiserror::Error)]
pub enum IngestError {
    #[error("RPC connection failed: {0}")]
    RpcConnect(SharedError),
    #[error("transaction fetch failed for {hash}: {source}")]
    TxFetch { hash: String, source: SharedError },
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
