//! Builder for wiring together the optional runtime-core and process shutdown hooks.

use anyhow::Result;
use runtime_core::{RuntimeCore, RuntimeCoreHandle, RuntimeCoreStartArgs};

use crate::handle::{NodeRuntime, ShutdownHook};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Ingest mode requested for the node runtime.
pub enum IngestMode {
    Rpc,
    P2p,
    Hybrid,
}

impl IngestMode {
    /// Returns the stable string label used by config and diagnostics.
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

type StartupHookFactory =
    dyn FnOnce(Option<RuntimeCoreHandle>) -> Result<Option<ShutdownHook>> + Send;

#[derive(Default)]
/// Configures how the outer node runtime is started and shut down.
pub struct NodeRuntimeBuilder {
    ingest_mode: IngestMode,
    runtime_core_start_args: Option<RuntimeCoreStartArgs>,
    startup: Option<Box<StartupHookFactory>>,
}

impl NodeRuntimeBuilder {
    /// Builds a runtime builder from environment-backed ingest settings.
    pub fn from_env() -> Result<Self> {
        let ingest_mode = IngestMode::parse(std::env::var("VIZ_API_INGEST_MODE").ok().as_deref());
        Ok(Self {
            ingest_mode,
            runtime_core_start_args: None,
            startup: None,
        })
    }

    /// Returns the ingest mode selected for this runtime.
    pub fn ingest_mode(&self) -> IngestMode {
        self.ingest_mode
    }

    /// Attaches the arguments needed to start `runtime-core`.
    pub fn with_runtime_core_start_args(mut self, args: RuntimeCoreStartArgs) -> Self {
        self.runtime_core_start_args = Some(args);
        self
    }

    /// Registers an additional startup hook that can return a matching shutdown hook.
    pub fn with_startup<F>(mut self, startup: F) -> Self
    where
        F: FnOnce(Option<RuntimeCoreHandle>) -> Result<Option<ShutdownHook>> + Send + 'static,
    {
        self.startup = Some(Box::new(startup));
        self
    }

    /// Starts the configured components and returns a runtime handle.
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
