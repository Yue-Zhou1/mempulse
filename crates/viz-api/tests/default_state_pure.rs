use runtime_core::{RuntimeCore, RuntimeIngestMode};
use viz_api::{
    RuntimeCoreViewProviders, app_state_from_runtime_bootstrap, default_runtime_bootstrap,
};

#[tokio::test]
async fn app_state_projection_is_pure_and_does_not_spawn_ingest() {
    let bootstrap = default_runtime_bootstrap();
    let runtime_core =
        RuntimeCore::start(bootstrap.runtime_core_start_args(RuntimeIngestMode::Rpc))
            .expect("runtime core should start");

    let _state = app_state_from_runtime_bootstrap(
        &bootstrap,
        RuntimeCoreViewProviders::from_runtime_core(runtime_core.clone()),
    );

    assert_eq!(runtime_core.live_rpc_feed_start_count(), 0);
}
