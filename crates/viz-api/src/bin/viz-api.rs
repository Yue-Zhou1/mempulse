#![forbid(unsafe_code)]

use anyhow::Result;
use node_runtime::{IngestMode, NodeRuntimeBuilder};
use runtime_core::RuntimeIngestMode;
use runtime_core::live_rpc::{
    start_live_rpc_feed_with_runtime_core, start_live_rpc_pending_pool_rebuild_with_runtime_core,
};
use std::env;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use viz_api::{
    RuntimeCoreViewProviders, app_state_from_runtime_bootstrap, build_router,
    default_runtime_bootstrap,
};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing(env::var("RUST_LOG").ok().as_deref());
    let bootstrap = default_runtime_bootstrap();
    let state_bootstrap = bootstrap.clone();
    let runtime_builder = NodeRuntimeBuilder::from_env()?;
    let ingest_mode = runtime_builder.ingest_mode();
    let (runtime_core_start_args, startup) =
        bootstrap.into_runtime_startup(runtime_ingest_mode(ingest_mode));
    let live_rpc_config = startup.live_rpc_config.clone();
    let rebuild_scheduler_from_rpc = startup.rebuild_scheduler_from_rpc;
    let runtime = runtime_builder
        .with_runtime_core_start_args(runtime_core_start_args)
        .with_startup(move |runtime_core| {
            let runtime_core = runtime_core.expect("runtime core handle");
            if matches!(ingest_mode, IngestMode::Rpc | IngestMode::Hybrid) {
                start_live_rpc_feed_with_runtime_core(
                    runtime_core.clone(),
                    live_rpc_config.clone(),
                    rebuild_scheduler_from_rpc,
                );
            } else if rebuild_scheduler_from_rpc {
                start_live_rpc_pending_pool_rebuild_with_runtime_core(
                    runtime_core,
                    live_rpc_config.clone(),
                );
            } else {
                tracing::info!(
                    ingest_mode = %ingest_mode.as_str(),
                    "ingest mode does not start live rpc feed"
                );
            }
            Ok(None)
        })
        .build()?;
    let runtime_core = runtime.runtime_core().expect("runtime core handle");
    startup.start_background_tasks(runtime_core.clone());
    let state = app_state_from_runtime_bootstrap(
        &state_bootstrap,
        RuntimeCoreViewProviders::from_runtime_core(runtime_core.clone()),
    );
    let app = build_router(state);
    let bind_addr = resolve_bind_addr(env::var("VIZ_API_BIND").ok().as_deref());
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!(
        bind_addr = %bind_addr,
        ingest_mode = %ingest_mode.as_str(),
        "viz-api listening"
    );
    axum::serve(listener, app).await?;
    startup.abort_background_tasks();
    runtime.shutdown().await?;
    Ok(())
}

fn runtime_ingest_mode(mode: IngestMode) -> RuntimeIngestMode {
    match mode {
        IngestMode::Rpc => RuntimeIngestMode::Rpc,
        IngestMode::P2p => RuntimeIngestMode::P2p,
        IngestMode::Hybrid => RuntimeIngestMode::Hybrid,
    }
}

fn resolve_bind_addr(env_override: Option<&str>) -> String {
    match env_override.map(str::trim) {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => "0.0.0.0:3000".to_owned(),
    }
}

fn init_tracing(log_override: Option<&str>) {
    let filter = resolve_log_filter(log_override);
    let env_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init();
}

fn resolve_log_filter(env_override: Option<&str>) -> String {
    match env_override.map(str::trim) {
        Some(value) if !value.is_empty() => {
            let has_viz_api_directive = value
                .split(',')
                .map(str::trim)
                .any(|directive| directive == "viz_api" || directive.starts_with("viz_api="));
            if has_viz_api_directive {
                value.to_owned()
            } else {
                format!("{value},viz_api=info")
            }
        }
        _ => "info,viz_api=info".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolve_bind_addr_defaults_to_all_interfaces() {
        let bind = super::resolve_bind_addr(None);
        assert_eq!(bind, "0.0.0.0:3000");
    }

    #[test]
    fn resolve_bind_addr_uses_env_override() {
        let bind = super::resolve_bind_addr(Some("127.0.0.1:9000"));
        assert_eq!(bind, "127.0.0.1:9000");
    }

    #[test]
    fn runtime_ingest_mode_matches_node_runtime_mode() {
        assert_eq!(
            super::runtime_ingest_mode(node_runtime::IngestMode::Rpc),
            runtime_core::RuntimeIngestMode::Rpc
        );
        assert_eq!(
            super::runtime_ingest_mode(node_runtime::IngestMode::P2p),
            runtime_core::RuntimeIngestMode::P2p
        );
        assert_eq!(
            super::runtime_ingest_mode(node_runtime::IngestMode::Hybrid),
            runtime_core::RuntimeIngestMode::Hybrid
        );
    }

    #[test]
    fn resolve_log_filter_defaults_to_info_for_viz_api() {
        let filter = super::resolve_log_filter(None);
        assert_eq!(filter, "info,viz_api=info");
    }

    #[test]
    fn resolve_log_filter_uses_env_override() {
        let filter = super::resolve_log_filter(Some("warn,viz_api=debug"));
        assert_eq!(filter, "warn,viz_api=debug");
    }

    #[test]
    fn resolve_log_filter_appends_viz_api_info_when_missing() {
        let filter = super::resolve_log_filter(Some("warn"));
        assert_eq!(filter, "warn,viz_api=info");
    }
}
