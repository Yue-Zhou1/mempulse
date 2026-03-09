use std::fs;
use std::path::PathBuf;

#[test]
fn viz_api_live_rpc_facade_no_longer_contains_runtime_implementation() {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/live_rpc.rs"))
            .expect("read live_rpc source");

    for forbidden in [
        "LegacyGlobals",
        "LIVE_RPC_DROP_METRICS",
        "LIVE_RPC_SEARCHER_METRICS",
        "LIVE_RPC_SIMULATION_METRICS",
        "LIVE_RPC_SIMULATION_STATUS",
        "LIVE_RPC_BUILDER_ENGINE",
        "LIVE_RPC_CHAIN_STATUS",
        "LIVE_RPC_SIMULATION_CACHE",
        "LIVE_RPC_SIMULATION_HTTP_CLIENT",
        "LIVE_RPC_SIMULATION_SERVICE",
        "LIVE_RPC_MONO_EPOCH",
        "reset_live_rpc_simulation_runtime_state",
        "reset_live_rpc_builder_runtime_state",
        "reset_live_rpc_simulation_cache",
        "live_rpc_drop_metrics_snapshot",
        "live_rpc_searcher_metrics_snapshot",
        "live_rpc_simulation_metrics_snapshot",
        "live_rpc_simulation_status_snapshot",
        "live_rpc_builder_snapshot",
        "live_rpc_builder_metrics_snapshot",
        "live_rpc_chain_status_snapshot",
        "start_live_rpc_feed_with_runtime_core",
        "start_live_rpc_pending_pool_rebuild_with_runtime_core",
        "start_live_rpc_feed_with_owner",
        "start_live_rpc_pending_pool_rebuild_with_owner",
        "run_chain_worker",
        "run_chain_pending_pool_rebuild",
    ] {
        assert!(
            !source.contains(forbidden),
            "live_rpc.rs still contains legacy runtime ownership marker: {forbidden}"
        );
    }
}

#[test]
fn viz_api_lib_no_longer_contains_process_global_runtime_state() {
    let source = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
        .expect("read viz-api lib source");

    for forbidden in [
        "SCHEDULER_SNAPSHOT_MONO_EPOCH",
        "dashboard_stream_broadcaster_registry",
        "OnceLock<Mutex<HashMap<usize, Arc<DashboardStreamBroadcaster>>>>",
    ] {
        assert!(
            !source.contains(forbidden),
            "viz-api lib still contains process-global runtime state marker: {forbidden}"
        );
    }
}

#[test]
fn runtime_core_live_rpc_no_longer_contains_process_global_feed_state() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime-core/src/live_rpc.rs"),
    )
    .expect("read runtime-core live_rpc source");

    for forbidden in [
        "LIVE_RPC_FEED_START_COUNT",
        "pub fn live_rpc_feed_start_count(",
        "pub fn reset_live_rpc_feed_start_count(",
        "LIVE_RPC_TEST_MUTEX",
        "live_rpc_test_guard(",
    ] {
        assert!(
            !source.contains(forbidden),
            "runtime-core live_rpc still contains process-global feed state marker: {forbidden}"
        );
    }
}

#[test]
fn viz_api_binary_no_longer_imports_runtime_startup_from_viz_api_live_rpc() {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bin/viz-api.rs"))
            .expect("read viz-api binary source");

    for forbidden in [
        "use viz_api::live_rpc::{",
        "viz_api::live_rpc::start_live_rpc_feed_with_runtime_core",
        "viz_api::live_rpc::start_live_rpc_pending_pool_rebuild_with_runtime_core",
    ] {
        assert!(
            !source.contains(forbidden),
            "viz-api binary still imports runtime startup from viz_api::live_rpc: {forbidden}"
        );
    }
}

#[test]
fn viz_api_binary_aborts_background_tasks_before_runtime_shutdown() {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bin/viz-api.rs"))
            .expect("read viz-api binary source");

    let abort_index = source
        .find("abort_background_tasks")
        .expect("binary should abort background tasks");
    let shutdown_index = source
        .find("runtime.shutdown().await")
        .expect("binary should shut down runtime");

    assert!(
        abort_index < shutdown_index,
        "viz-api binary shuts runtime down before aborting background tasks"
    );
}
