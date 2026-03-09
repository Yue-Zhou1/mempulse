use bench::measure_storage_snapshot_latency;
use std::fs;
use std::path::{Path, PathBuf};

const ENV_ARTIFACT_PATH: &str = "BENCH_STORAGE_SNAPSHOT_PERF_ARTIFACT";
const ENV_PENDING_COUNT: &str = "BENCH_STORAGE_SNAPSHOT_PENDING_COUNT";
const ENV_TAIL_EVENT_COUNT: &str = "BENCH_STORAGE_SNAPSHOT_TAIL_EVENT_COUNT";
const ENV_ITERATIONS: &str = "BENCH_STORAGE_SNAPSHOT_ITERATIONS";
const DEFAULT_ARTIFACT_PATH: &str = "artifacts/perf/storage_snapshot.json";
const DEFAULT_PENDING_COUNT: usize = 256;
const DEFAULT_TAIL_EVENT_COUNT: usize = 32;
const DEFAULT_ITERATIONS: usize = 40;

#[test]
fn storage_snapshot_perf_emits_metrics_artifact() {
    let pending_count = read_env_usize(ENV_PENDING_COUNT, DEFAULT_PENDING_COUNT);
    let tail_event_count = read_env_usize(ENV_TAIL_EVENT_COUNT, DEFAULT_TAIL_EVENT_COUNT);
    let iterations = read_env_usize(ENV_ITERATIONS, DEFAULT_ITERATIONS);
    let artifact_path = artifact_path_from_env();

    let report = measure_storage_snapshot_latency(pending_count, tail_event_count, iterations);
    let report_stdout = serde_json::to_string(&report).expect("serialize storage snapshot report");
    println!("STORAGE_SNAPSHOT_PERF={report_stdout}");

    write_artifact(&artifact_path, &report_stdout);
    let artifact_contents =
        fs::read_to_string(&artifact_path).expect("read storage snapshot artifact");
    let persisted: serde_json::Value =
        serde_json::from_str(&artifact_contents).expect("decode storage snapshot artifact");

    assert_eq!(
        persisted["pending_count"].as_u64(),
        Some(pending_count as u64)
    );
    assert_eq!(
        persisted["tail_event_count"].as_u64(),
        Some(tail_event_count as u64)
    );
    assert_eq!(
        persisted["replay_event_count"].as_u64(),
        Some(tail_event_count as u64)
    );
    assert_eq!(persisted["snapshot_present"].as_bool(), Some(true));
    assert_eq!(persisted["iterations"].as_u64(), Some(iterations as u64));
    assert!(persisted["write_p95_us"].as_u64().is_some());
    assert!(persisted["rehydrate_p95_us"].as_u64().is_some());
}

fn artifact_path_from_env() -> PathBuf {
    std::env::var_os(ENV_ARTIFACT_PATH)
        .map(PathBuf::from)
        .map(resolve_workspace_path)
        .unwrap_or_else(|| resolve_workspace_path(PathBuf::from(DEFAULT_ARTIFACT_PATH)))
}

fn read_env_usize(name: &str, default_value: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default_value)
        .max(1)
}

fn write_artifact(path: &Path, payload: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create storage snapshot artifact directory");
    }
    fs::write(path, payload).expect("write storage snapshot artifact");
}

fn resolve_workspace_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        workspace_root().join(path)
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("bench crate has workspace root")
        .to_path_buf()
}
