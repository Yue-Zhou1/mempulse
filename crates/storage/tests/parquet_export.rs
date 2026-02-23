use replay::ReplayFrame;
use std::time::{SystemTime, UNIX_EPOCH};
use storage::{ArrowParquetExporter, ParquetExporter};

fn hash(v: u8) -> [u8; 32] {
    [v; 32]
}

fn temp_path() -> std::path::PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("prototype03-replay-{now}.parquet"))
}

#[test]
fn parquet_export_writes_replay_frames_that_can_be_read_back() {
    let frames = vec![
        ReplayFrame {
            seq_hi: 1,
            timestamp_unix_ms: 1_700_000_000_001,
            pending: vec![hash(1)],
        },
        ReplayFrame {
            seq_hi: 2,
            timestamp_unix_ms: 1_700_000_000_002,
            pending: vec![hash(2), hash(3)],
        },
    ];
    let path = temp_path();
    let exporter = ArrowParquetExporter;
    exporter
        .export_replay_frames_parquet(&frames, path.to_str().expect("utf8 path"))
        .expect("export replay frames");

    let bytes = std::fs::read(&path).expect("read export file");
    let decoded: Vec<ReplayFrame> = serde_json::from_slice(&bytes).expect("decode exported file");
    assert_eq!(decoded, frames);

    let _ = std::fs::remove_file(path);
}
