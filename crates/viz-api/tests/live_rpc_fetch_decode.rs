use viz_api::live_rpc::decode_transaction_fetch_response;

#[test]
fn live_rpc_fetch_decode_decodes_tx_from_raw_bytes() {
    let hash_hex = format!("0x{}", "11".repeat(32));
    let payload = format!(
        r#"{{
          "jsonrpc":"2.0",
          "id":42,
          "result":{{
            "hash":"{hash_hex}",
            "from":"0x{from}",
            "to":"0x{to}",
            "nonce":"0x2a",
            "type":"0x2",
            "input":"0xaabb",
            "chainId":"0x1"
          }}
        }}"#,
        from = "22".repeat(20),
        to = "33".repeat(20),
    );

    let tx = decode_transaction_fetch_response(payload.as_bytes(), &hash_hex)
        .expect("decode bytes payload")
        .expect("transaction present");

    assert_eq!(tx.hash, [0x11; 32]);
    assert_eq!(tx.sender, [0x22; 20]);
    assert_eq!(tx.to, Some([0x33; 20]));
    assert_eq!(tx.nonce, 42);
    assert_eq!(tx.tx_type, 2);
    assert_eq!(tx.chain_id, Some(1));
    assert_eq!(tx.calldata_len, Some(2));
}

#[test]
fn live_rpc_fetch_decode_returns_none_for_null_result() {
    let payload = br#"{"jsonrpc":"2.0","id":42,"result":null}"#;
    let tx = decode_transaction_fetch_response(payload, "0x00").expect("decode response");
    assert!(tx.is_none());
}

#[test]
fn live_rpc_fetch_decode_preserves_rpc_error_behavior() {
    let payload = br#"{"jsonrpc":"2.0","id":42,"error":{"code":-32000,"message":"boom"}}"#;
    let err = decode_transaction_fetch_response(payload, "0x00").expect_err("rpc error");
    assert!(err.to_string().contains("rpc returned error"));
}
