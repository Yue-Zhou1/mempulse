use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Router, serve};
use builder::{BlockTemplate, RelayClient, RelayClientConfig, RelayDryRunResult};
use serde::Serialize;
use serde_json::json;
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;

#[derive(Clone)]
struct MockRelayState {
    requests: Arc<AtomicUsize>,
}

#[derive(Debug, Serialize)]
struct RelayTraceSummary {
    generated_unix_ms: i64,
    scenarios: Vec<RelayScenarioSummary>,
}

#[derive(Debug, Serialize)]
struct RelayScenarioSummary {
    name: String,
    accepted: bool,
    final_state: String,
    attempt_count: usize,
    had_http_5xx: bool,
    had_transport_error: bool,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let out_dir = resolve_out_dir();
    fs::create_dir_all(&out_dir)?;

    let flaky_success = run_flaky_success_scenario().await?;
    let exhausted_failure = run_exhausted_failure_scenario().await?;

    fs::write(
        out_dir.join("relay_trace_flaky_success.json"),
        serde_json::to_vec_pretty(&flaky_success)?,
    )?;
    fs::write(
        out_dir.join("relay_trace_exhausted_failure.json"),
        serde_json::to_vec_pretty(&exhausted_failure)?,
    )?;

    let summary = RelayTraceSummary {
        generated_unix_ms: unix_ms_now(),
        scenarios: vec![
            scenario_summary("flaky_success", &flaky_success),
            scenario_summary("exhausted_failure", &exhausted_failure),
        ],
    };
    fs::write(
        out_dir.join("relay_trace_summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;

    println!(
        "relay-trace-package: out_dir={} scenarios={}",
        out_dir.display(),
        summary.scenarios.len()
    );
    Ok(())
}

async fn run_flaky_success_scenario() -> anyhow::Result<RelayDryRunResult> {
    let state = MockRelayState {
        requests: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route("/relay/v1/builder/blocks", post(mock_relay_flaky))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let _ = serve(listener, app).await;
    });

    let client = RelayClient::new(RelayClientConfig {
        relay_url: format!("http://{addr}/relay/v1/builder/blocks"),
        max_retries: 2,
        initial_backoff_ms: 10,
        request_timeout_ms: 2_000,
    })?;
    let result = client.submit_dry_run(&mock_template()).await?;
    server.abort();
    Ok(result)
}

async fn run_exhausted_failure_scenario() -> anyhow::Result<RelayDryRunResult> {
    let client = RelayClient::new(RelayClientConfig {
        relay_url: "http://127.0.0.1:9/relay/v1/builder/blocks".to_owned(),
        max_retries: 2,
        initial_backoff_ms: 10,
        request_timeout_ms: 300,
    })?;
    let result = client.submit_dry_run(&mock_template()).await?;
    Ok(result)
}

fn scenario_summary(name: &str, result: &RelayDryRunResult) -> RelayScenarioSummary {
    RelayScenarioSummary {
        name: name.to_owned(),
        accepted: result.accepted,
        final_state: result.final_state.clone(),
        attempt_count: result.attempts.len(),
        had_http_5xx: result
            .attempts
            .iter()
            .filter_map(|attempt| attempt.http_status)
            .any(|status| status >= 500),
        had_transport_error: result
            .attempts
            .iter()
            .any(|attempt| attempt.error.is_some()),
    }
}

fn resolve_out_dir() -> PathBuf {
    let args = env::args().collect::<Vec<_>>();
    let mut idx = 1;
    while idx < args.len() {
        if args[idx] == "--out-dir" && idx + 1 < args.len() {
            return PathBuf::from(&args[idx + 1]);
        }
        idx += 1;
    }
    PathBuf::from("artifacts/builder")
}

fn mock_template() -> BlockTemplate {
    BlockTemplate {
        slot: 123_456,
        parent_hash: "0x1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
        block_hash: "0x2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
        builder_pubkey: "0x3333333333333333333333333333333333333333333333333333333333333333".to_owned(),
        tx_count: 14,
        gas_used: 18_900_000,
    }
}

async fn mock_relay_flaky(State(state): State<MockRelayState>) -> (StatusCode, Json<serde_json::Value>) {
    let n = state.requests.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "temporary relay outage" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "relay": "mock-flaky"
        })),
    )
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
