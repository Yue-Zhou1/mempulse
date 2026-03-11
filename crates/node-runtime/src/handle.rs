//! Runtime handle and shutdown hook integration points.

use anyhow::Result;
use runtime_core::RuntimeCoreHandle;
use std::sync::Mutex;

/// One-shot cleanup hook executed before the underlying runtime core shuts down.
pub type ShutdownHook = Box<dyn FnOnce() -> Result<()> + Send>;

/// Handle returned by `NodeRuntimeBuilder` for accessing runtime-core and coordinated shutdown.
pub struct NodeRuntime {
    runtime_core: Option<RuntimeCoreHandle>,
    shutdown: Mutex<Option<ShutdownHook>>,
}

impl NodeRuntime {
    /// Creates a runtime handle from an optional runtime-core and shutdown hook.
    pub fn new(runtime_core: Option<RuntimeCoreHandle>, shutdown: Option<ShutdownHook>) -> Self {
        Self {
            runtime_core,
            shutdown: Mutex::new(shutdown),
        }
    }

    /// Returns the underlying runtime-core handle when one was started.
    pub fn runtime_core(&self) -> Option<RuntimeCoreHandle> {
        self.runtime_core.clone()
    }

    /// Runs the registered shutdown hook and then shuts down runtime-core.
    pub async fn shutdown(self) -> Result<()> {
        let hook = self.shutdown.into_inner().ok().flatten();
        if let Some(hook) = hook {
            hook()?;
        }
        if let Some(runtime_core) = self.runtime_core {
            runtime_core.shutdown().await?;
        }
        Ok(())
    }
}
