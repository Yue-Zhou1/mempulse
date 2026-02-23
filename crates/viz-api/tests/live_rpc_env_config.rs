use viz_api::live_rpc::LiveRpcConfig;

const ENV_ETH_WS_URL: &str = "VIZ_API_ETH_WS_URL";
const ENV_ETH_HTTP_URL: &str = "VIZ_API_ETH_HTTP_URL";
const ENV_SOURCE_ID: &str = "VIZ_API_SOURCE_ID";
const ENV_MAX_SEEN_HASHES: &str = "VIZ_API_MAX_SEEN_HASHES";

#[test]
fn live_rpc_config_prefers_env_over_defaults() {
    unsafe {
        std::env::set_var(ENV_ETH_WS_URL, "wss://example/ws");
        std::env::set_var(ENV_ETH_HTTP_URL, "https://example/http");
        std::env::set_var(ENV_SOURCE_ID, "rpc-custom");
        std::env::set_var(ENV_MAX_SEEN_HASHES, "2048");
    }

    let config = LiveRpcConfig::from_env().expect("read live rpc config");
    assert_eq!(config.primary_ws_url(), Some("wss://example/ws"));
    assert_eq!(config.primary_http_url(), Some("https://example/http"));
    assert_eq!(config.source_id().to_string(), "rpc-custom");
    assert_eq!(config.max_seen_hashes(), 2048);

    unsafe {
        std::env::remove_var(ENV_ETH_WS_URL);
        std::env::remove_var(ENV_ETH_HTTP_URL);
        std::env::remove_var(ENV_SOURCE_ID);
        std::env::remove_var(ENV_MAX_SEEN_HASHES);
    }
}
