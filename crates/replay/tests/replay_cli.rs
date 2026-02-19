use common::{Address, SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn hash(v: u8) -> TxHash {
    [v; 32]
}

fn address(v: u8) -> Address {
    [v; 20]
}

fn temp_file(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!("prototype03-{name}-{nanos}.json"))
}

#[test]
fn replay_cli_writes_output_file() {
    let input_path = temp_file("in");
    let output_path = temp_file("out");

    let events = vec![EventEnvelope {
        seq_id: 1,
        ingest_ts_unix_ms: 1_700_000_000_000,
        ingest_ts_mono_ns: 10,
        source_id: SourceId::new("test"),
        payload: EventPayload::TxDecoded(TxDecoded {
            hash: hash(1),
            tx_type: 2,
            sender: address(9),
            nonce: 1,
        }),
    }];
    fs::write(
        &input_path,
        serde_json::to_vec(&events).expect("json events"),
    )
    .expect("write input");

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_replay-cli"))
        .args([
            "--input",
            input_path.to_str().expect("input path"),
            "--out",
            output_path.to_str().expect("output path"),
            "--mode",
            "deterministic",
        ])
        .status()
        .expect("run replay-cli");

    assert!(status.success());
    let output = fs::read_to_string(&output_path).expect("read output");
    assert!(output.contains("seq_hi"));

    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}
