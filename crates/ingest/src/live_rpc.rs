#[derive(Clone, Debug, Default)]
pub struct LiveRpcConfig;

pub struct LiveRpcRuntime {
    _config: LiveRpcConfig,
}

impl LiveRpcRuntime {
    pub fn new(config: LiveRpcConfig) -> Self {
        Self { _config: config }
    }
}
