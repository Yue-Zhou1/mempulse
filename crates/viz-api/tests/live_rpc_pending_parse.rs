use viz_api::live_rpc::parse_pending_hash;

#[test]
fn live_rpc_pending_parse_handles_subscription_handshake() {
    let mut subscription_id = None;
    let payload = r#"{"jsonrpc":"2.0","id":1,"result":"sub-123"}"#;
    let parsed = parse_pending_hash(payload, &mut subscription_id);

    assert!(parsed.is_none());
    assert_eq!(subscription_id.as_deref(), Some("sub-123"));
}

#[test]
fn live_rpc_pending_parse_borrows_hash_payload_when_subscription_matches() {
    let mut subscription_id = Some("sub-123".to_owned());
    let payload = r#"{
      "jsonrpc":"2.0",
      "method":"eth_subscription",
      "params":{"subscription":"sub-123","result":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
    }"#;

    let parsed = parse_pending_hash(payload, &mut subscription_id).expect("parse hash");

    assert_eq!(
        parsed,
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    let hash_ptr = parsed.as_ptr() as usize;
    let payload_start = payload.as_ptr() as usize;
    let payload_end = payload_start + payload.len();
    assert!(
        hash_ptr >= payload_start && hash_ptr < payload_end,
        "expected borrowed hash slice from payload"
    );
}

#[test]
fn live_rpc_pending_parse_ignores_non_matching_subscription_ids() {
    let mut subscription_id = Some("sub-expected".to_owned());
    let payload = r#"{
      "jsonrpc":"2.0",
      "method":"eth_subscription",
      "params":{"subscription":"sub-other","result":"0xbb"}
    }"#;

    assert!(parse_pending_hash(payload, &mut subscription_id).is_none());
}

#[test]
fn live_rpc_pending_parse_returns_none_for_malformed_payloads() {
    let mut subscription_id = None;
    let payload = r#"{"jsonrpc":"2.0","params":{"subscription":"sub-1"}"#;
    assert!(parse_pending_hash(payload, &mut subscription_id).is_none());
}
