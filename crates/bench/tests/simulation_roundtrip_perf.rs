use bench::measure_simulation_roundtrip_latency;
use std::fs;
use std::path::{Path, PathBuf};

const ENV_ARTIFACT_PATH: &str = "BENCH_SIMULATION_ROUNDTRIP_PERF_ARTIFACT";
const ENV_CANDIDATE_COUNT: &str = "BENCH_SIMULATION_ROUNDTRIP_CANDIDATE_COUNT";
const ENV_ITERATIONS: &str = "BENCH_SIMULATION_ROUNDTRIP_ITERATIONS";
const DEFAULT_ARTIFACT_PATH: &str = "artifacts/perf/simulation_roundtrip.json";
const DEFAULT_CANDIDATE_COUNT: usize = 128;
const DEFAULT_ITERATIONS: usize = 40;

#[test]
fn simulation_roundtrip_perf_emits_metrics_artifact() {
    let candidate_count = read_env_usize(ENV_CANDIDATE_COUNT, DEFAULT_CANDIDATE_COUNT);
    let iterations = read_env_usize(ENV_ITERATIONS, DEFAULT_ITERATIONS);
    let artifact_path = artifact_path_from_env();

    let report = measure_simulation_roundtrip_latency(candidate_count, iterations);
    let report_stdout =
        serde_json::to_string(&report).expect("serialize simulation roundtrip report");
    println!("SIMULATION_ROUNDTRIP_PERF={report_stdout}");

    write_artifact(&artifact_path, &report_stdout);
    let artifact_contents =
        fs::read_to_string(&artifact_path).expect("read simulation roundtrip artifact");
    let persisted: serde_json::Value =
        serde_json::from_str(&artifact_contents).expect("decode simulation roundtrip artifact");

    assert_eq!(
        persisted["candidate_count"].as_u64(),
        Some(candidate_count as u64)
    );
    assert_eq!(
        persisted["simulation_tasks"].as_u64(),
        Some(candidate_count as u64)
    );
    assert_eq!(
        persisted["handoff_total"].as_u64(),
        Some(candidate_count as u64)
    );
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
        fs::create_dir_all(parent).expect("create simulation roundtrip artifact directory");
    }
    fs::write(path, payload).expect("write simulation roundtrip artifact");
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
