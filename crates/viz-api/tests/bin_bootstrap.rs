use node_runtime::NodeRuntimeBuilder;
use viz_api::default_state_with_runtime;

#[tokio::test]
async fn binary_bootstrap_uses_runtime_builder_contract() {
    let (_state, _bootstrap) = default_state_with_runtime();
    let _builder = NodeRuntimeBuilder::from_env().expect("runtime builder");
}
