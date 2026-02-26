use anyhow::Result;

use crate::handle::{NodeRuntime, ShutdownHook};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestMode {
    Rpc,
    P2p,
    Hybrid,
}

impl IngestMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::P2p => "p2p",
            Self::Hybrid => "hybrid",
        }
    }

    fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase) {
            Some(mode) if mode == "p2p" => Self::P2p,
            Some(mode) if mode == "hybrid" => Self::Hybrid,
            _ => Self::Rpc,
        }
    }
}

impl Default for IngestMode {
    fn default() -> Self {
        Self::Rpc
    }
}

#[derive(Default)]
pub struct NodeRuntimeBuilder {
    ingest_mode: IngestMode,
    startup: Option<Box<dyn FnOnce() -> Result<Option<ShutdownHook>> + Send>>,
}

impl NodeRuntimeBuilder {
    pub fn from_env() -> Result<Self> {
        let ingest_mode = IngestMode::parse(std::env::var("VIZ_API_INGEST_MODE").ok().as_deref());
        Ok(Self {
            ingest_mode,
            startup: None,
        })
    }

    pub fn ingest_mode(&self) -> IngestMode {
        self.ingest_mode
    }

    pub fn with_startup<F>(mut self, startup: F) -> Self
    where
        F: FnOnce() -> Result<Option<ShutdownHook>> + Send + 'static,
    {
        self.startup = Some(Box::new(startup));
        self
    }

    pub fn build(self) -> Result<NodeRuntime> {
        let shutdown = match self.startup {
            Some(startup) => startup()?,
            None => None,
        };
        Ok(NodeRuntime::new(shutdown))
    }
}
