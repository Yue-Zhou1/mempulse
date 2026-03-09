use bench::measure_scheduler_pipeline_latency;
use std::fs;
use std::path::{Path, PathBuf};

const ENV_ARTIFACT_PATH: &str = "BENCH_SCHEDULER_PIPELINE_PERF_ARTIFACT";
const ENV_BATCH_SIZE: &str = "BENCH_SCHEDULER_PIPELINE_BATCH_SIZE";
const ENV_ITERATIONS: &str = "BENCH_SCHEDULER_PIPELINE_ITERATIONS";
const DEFAULT_ARTIFACT_PATH: &str = "artifacts/perf/scheduler_pipeline.json";
const DEFAULT_BATCH_SIZE: usize = 256;
const DEFAULT_ITERATIONS: usize = 40;

#[test]
fn scheduler_pipeline_perf_emits_metrics_artifact() {
    let batch_size = read_env_usize(ENV_BATCH_SIZE, DEFAULT_BATCH_SIZE);
    let iterations = read_env_usize(ENV_ITERATIONS, DEFAULT_ITERATIONS);
    let artifact_path = artifact_path_from_env();

    let report = measure_scheduler_pipeline_latency(batch_size, iterations);
    let report_stdout =
        serde_json::to_string(&report).expect("serialize scheduler pipeline report");
    println!("SCHEDULER_PIPELINE_PERF={report_stdout}");

    write_artifact(&artifact_path, &report_stdout);
    let artifact_contents =
        fs::read_to_string(&artifact_path).expect("read scheduler pipeline artifact");
    let persisted: serde_json::Value =
        serde_json::from_str(&artifact_contents).expect("decode scheduler pipeline artifact");

    assert_eq!(persisted["batch_size"].as_u64(), Some(batch_size as u64));
    assert_eq!(persisted["ready_total"].as_u64(), Some(batch_size as u64));
    assert_eq!(persisted["pending_total"].as_u64(), Some(batch_size as u64));
    assert_eq!(persisted["iterations"].as_u64(), Some(iterations as u64));
    assert!(persisted["p95_us"].as_u64().is_some());
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
        fs::create_dir_all(parent).expect("create scheduler pipeline artifact directory");
    }
    fs::write(path, payload).expect("write scheduler pipeline artifact");
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
