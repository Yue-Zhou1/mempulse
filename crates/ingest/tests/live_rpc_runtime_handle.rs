use ingest::{LiveRpcConfig, LiveRpcRuntime};

#[test]
fn runtime_start_returns_handle() {
    let _ = LiveRpcRuntime::new(LiveRpcConfig);
}
