use anyhow::Result;
use std::sync::Mutex;

pub type ShutdownHook = Box<dyn FnOnce() -> Result<()> + Send>;

pub struct NodeRuntime {
    shutdown: Mutex<Option<ShutdownHook>>,
}

impl NodeRuntime {
    pub fn new(shutdown: Option<ShutdownHook>) -> Self {
        Self {
            shutdown: Mutex::new(shutdown),
        }
    }

    pub async fn shutdown(self) -> Result<()> {
        let hook = self.shutdown.into_inner().ok().flatten();
        if let Some(hook) = hook {
            hook()?;
        }
        Ok(())
    }
}
