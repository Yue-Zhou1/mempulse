use common::{Address, SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded};

fn hash(value: u8) -> TxHash {
    [value; 32]
}

fn address(value: u8) -> Address {
    [value; 20]
}

#[test]
fn tx_schema_roundtrip_preserves_extended_fields() {
    let envelope = EventEnvelope {
        seq_id: 42,
        ingest_ts_unix_ms: 1_700_000_123_456,
        ingest_ts_mono_ns: 9_999_999,
        source_id: SourceId::new("schema-test"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash(0xaa),
            tx_type: 2,
            sender: address(0xbb),
            nonce: 17,
            chain_id: Some(1),
            to: Some(address(0xcc)),
            value_wei: Some(1_000_000_000_000_000_000),
            gas_limit: Some(180_000),
            gas_price_wei: Some(45_000_000_000),
            max_fee_per_gas_wei: Some(60_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: Some(3),
            calldata_len: Some(196),
        }),
    };

    let encoded = serde_json::to_vec(&envelope).expect("serialize envelope");
    let decoded: EventEnvelope = serde_json::from_slice(&encoded).expect("deserialize envelope");

    assert_eq!(decoded, envelope);
}
