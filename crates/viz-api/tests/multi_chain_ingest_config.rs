use viz_api::live_rpc::{LiveRpcConfig, resolve_record_chain_id, worker_count_for_config};

const ENV_CHAINS: &str = "VIZ_API_CHAINS";
const ENV_MAX_SEEN_HASHES: &str = "VIZ_API_MAX_SEEN_HASHES";

#[test]
fn multi_chain_ingest_config_parses_multiple_chains_and_spawns_workers() {
    unsafe {
        std::env::set_var(
            ENV_CHAINS,
            r#"[
              {
                "chain_key": "eth-mainnet",
                "chain_id": 1,
                "ws_url": "wss://eth.example/ws",
                "http_url": "https://eth.example/http",
                "source_id": "rpc-eth-mainnet"
              },
              {
                "chain_key": "base-mainnet",
                "chain_id": 8453,
                "ws_url": "wss://base.example/ws",
                "http_url": "https://base.example/http",
                "source_id": "rpc-base-mainnet"
              }
            ]"#,
        );
        std::env::set_var(ENV_MAX_SEEN_HASHES, "4096");
    }

    let config = LiveRpcConfig::from_env().expect("parse multi-chain live rpc config");
    assert_eq!(config.chain_configs().len(), 2);
    assert_eq!(config.max_seen_hashes(), 4096);

    let chain0 = &config.chain_configs()[0];
    assert_eq!(chain0.chain_key(), "eth-mainnet");
    assert_eq!(chain0.chain_id(), Some(1));
    assert_eq!(chain0.source_id().to_string(), "rpc-eth-mainnet");

    let chain1 = &config.chain_configs()[1];
    assert_eq!(chain1.chain_key(), "base-mainnet");
    assert_eq!(chain1.chain_id(), Some(8453));
    assert_eq!(chain1.source_id().to_string(), "rpc-base-mainnet");

    assert_eq!(worker_count_for_config(&config), 2);

    unsafe {
        std::env::remove_var(ENV_CHAINS);
        std::env::remove_var(ENV_MAX_SEEN_HASHES);
    }
}

#[test]
fn multi_chain_ingest_config_propagates_chain_tags_to_storage_records() {
    assert_eq!(resolve_record_chain_id(Some(1), None), Some(1));
    assert_eq!(resolve_record_chain_id(Some(1), Some(8453)), Some(8453));
    assert_eq!(resolve_record_chain_id(None, None), None);
}
