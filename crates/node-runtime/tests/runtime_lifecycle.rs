use node_runtime::NodeRuntimeBuilder;

#[tokio::test]
async fn runtime_exposes_shutdown_handle() {
    let runtime = NodeRuntimeBuilder::default().build().expect("build");
    runtime.shutdown().await.expect("shutdown");
}
