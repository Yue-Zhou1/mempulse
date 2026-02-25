use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use viz_api::live_rpc::LiveRpcConfig;

const ENV_CHAIN_CONFIG_PATH: &str = "VIZ_API_CHAIN_CONFIG_PATH";

#[test]
fn live_rpc_config_reads_multi_chain_json_file() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("viz-api-chain-config-{unique}.json"));
    fs::write(
        &path,
        r#"{
          "chains": [
            {
              "chain_key": "eth-mainnet",
              "chain_id": 1,
              "endpoints": [
                {
                  "ws_url": "wss://eth.example/ws",
                  "http_url": "https://eth.example/http"
                },
                {
                  "ws_url": "wss://eth-fallback.example/ws",
                  "http_url": "https://eth-fallback.example/http"
                }
              ],
              "source_id": "rpc-eth-mainnet"
            },
            {
              "chain_key": "base-mainnet",
              "chain_id": 8453,
              "ws_url": "wss://base.example/ws",
              "http_url": "https://base.example/http",
              "source_id": "rpc-base-mainnet"
            }
          ]
        }"#,
    )
    .expect("write chain config file");

    unsafe {
        std::env::set_var(ENV_CHAIN_CONFIG_PATH, path.to_string_lossy().as_ref());
    }

    let config = LiveRpcConfig::from_env().expect("load chain config from json file");
    assert_eq!(config.chain_configs().len(), 2);
    assert_eq!(config.chain_configs()[0].chain_key(), "eth-mainnet");
    assert_eq!(
        config.chain_configs()[0].primary_http_url(),
        Some("https://eth.example/http")
    );
    assert_eq!(config.chain_configs()[1].chain_id(), Some(8453));

    unsafe {
        std::env::remove_var(ENV_CHAIN_CONFIG_PATH);
    }
    let _ = fs::remove_file(path);
}
