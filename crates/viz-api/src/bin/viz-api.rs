use anyhow::Result;
use std::env;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use viz_api::{build_router, default_state};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing(env::var("RUST_LOG").ok().as_deref());
    let ingest_mode = resolve_ingest_mode(env::var("VIZ_API_INGEST_MODE").ok().as_deref());
    let app = build_router(default_state());
    let bind_addr = resolve_bind_addr(env::var("VIZ_API_BIND").ok().as_deref());
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!(bind_addr = %bind_addr, ingest_mode = %ingest_mode, "viz-api listening");
    axum::serve(listener, app).await?;
    Ok(())
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

fn resolve_ingest_mode(env_override: Option<&str>) -> String {
    match env_override.map(str::trim).map(str::to_ascii_lowercase) {
        Some(mode) if mode == "p2p" => "p2p".to_owned(),
        Some(mode) if mode == "hybrid" => "hybrid".to_owned(),
        _ => "rpc".to_owned(),
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

    #[test]
    fn resolve_ingest_mode_defaults_to_rpc() {
        let mode = super::resolve_ingest_mode(None);
        assert_eq!(mode, "rpc");
    }

    #[test]
    fn resolve_ingest_mode_accepts_valid_values() {
        assert_eq!(super::resolve_ingest_mode(Some("p2p")), "p2p");
        assert_eq!(super::resolve_ingest_mode(Some("hybrid")), "hybrid");
    }
}
