use feature_engine::registry::ProtocolRegistry;

#[test]
fn classifies_protocol_from_registry_json_without_code_changes() {
    let registry = ProtocolRegistry::from_json(
        r#"{
          "protocols":[{"name":"curve-v2","addresses":["0x1212121212121212121212121212121212121212"]}],
          "selector_protocols":[]
        }"#,
    )
    .expect("registry parses");

    let protocol = registry.classify(Some([0x12; 20]), Some([0xde, 0xad, 0xbe, 0xef]));
    assert_eq!(protocol, "curve-v2");
}

#[test]
fn falls_back_to_selector_mapping_when_address_is_missing() {
    let registry = ProtocolRegistry::from_json(
        r#"{
          "protocols":[],
          "selector_protocols":[{"selector":"0xa9059cbb","protocol":"erc20"}]
        }"#,
    )
    .expect("registry parses");

    let protocol = registry.classify(None, Some([0xa9, 0x05, 0x9c, 0xbb]));
    assert_eq!(protocol, "erc20");
}
