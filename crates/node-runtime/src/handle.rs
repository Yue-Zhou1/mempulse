use anyhow::Result;
use runtime_core::RuntimeCoreHandle;
use std::sync::Mutex;

pub type ShutdownHook = Box<dyn FnOnce() -> Result<()> + Send>;

pub struct NodeRuntime {
    runtime_core: Option<RuntimeCoreHandle>,
    shutdown: Mutex<Option<ShutdownHook>>,
}

impl NodeRuntime {
    pub fn new(runtime_core: Option<RuntimeCoreHandle>, shutdown: Option<ShutdownHook>) -> Self {
        Self {
            runtime_core,
            shutdown: Mutex::new(shutdown),
        }
    }

    pub fn runtime_core(&self) -> Option<RuntimeCoreHandle> {
        self.runtime_core.clone()
    }

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
