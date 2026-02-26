use ingest::{LiveRpcConfig, LiveRpcRuntime};

#[test]
fn runtime_state_is_instance_scoped() {
    let a = LiveRpcRuntime::new(LiveRpcConfig);
    let b = LiveRpcRuntime::new(LiveRpcConfig);
    assert_ne!(a.instance_id(), b.instance_id());
}
