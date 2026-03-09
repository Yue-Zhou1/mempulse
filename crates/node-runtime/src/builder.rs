use anyhow::Result;
use runtime_core::{RuntimeCore, RuntimeCoreHandle, RuntimeCoreStartArgs};

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
    runtime_core_start_args: Option<RuntimeCoreStartArgs>,
    startup:
        Option<Box<dyn FnOnce(Option<RuntimeCoreHandle>) -> Result<Option<ShutdownHook>> + Send>>,
}

impl NodeRuntimeBuilder {
    pub fn from_env() -> Result<Self> {
        let ingest_mode = IngestMode::parse(std::env::var("VIZ_API_INGEST_MODE").ok().as_deref());
        Ok(Self {
            ingest_mode,
            runtime_core_start_args: None,
            startup: None,
        })
    }

    pub fn ingest_mode(&self) -> IngestMode {
        self.ingest_mode
    }

    pub fn with_runtime_core_start_args(mut self, args: RuntimeCoreStartArgs) -> Self {
        self.runtime_core_start_args = Some(args);
        self
    }

    pub fn with_startup<F>(mut self, startup: F) -> Self
    where
        F: FnOnce(Option<RuntimeCoreHandle>) -> Result<Option<ShutdownHook>> + Send + 'static,
    {
        self.startup = Some(Box::new(startup));
        self
    }

    pub fn build(self) -> Result<NodeRuntime> {
        let runtime_core = self
            .runtime_core_start_args
            .map(RuntimeCore::start)
            .transpose()?;
        let shutdown = match self.startup {
            Some(startup) => startup(runtime_core.clone())?,
            None => None,
        };
        Ok(NodeRuntime::new(runtime_core, shutdown))
    }
}
