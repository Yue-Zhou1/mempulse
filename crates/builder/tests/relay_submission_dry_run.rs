use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Router, serve};
use builder::{BlockTemplate, RelayClient, RelayClientConfig, RelayDryRunResult};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;

#[derive(Clone)]
struct MockRelayState {
    requests: Arc<AtomicUsize>,
}

#[tokio::test]
async fn relay_submission_dry_run_retries_and_records_trace() {
    let state = MockRelayState {
        requests: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route("/relay/v1/builder/blocks", post(mock_relay))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        serve(listener, app).await.unwrap();
    });

    let client = RelayClient::new(RelayClientConfig {
        relay_url: format!("http://{addr}/relay/v1/builder/blocks"),
        max_retries: 2,
        initial_backoff_ms: 1,
        request_timeout_ms: 2_000,
    })
    .expect("client");
    let template = BlockTemplate {
        slot: 1234,
        parent_hash: "0x11".to_owned(),
        block_hash: "0x22".to_owned(),
        builder_pubkey: "0x33".to_owned(),
        tx_count: 12,
        gas_used: 18_500_000,
    };

    let result: RelayDryRunResult = client.submit_dry_run(&template).await.expect("dry-run");

    assert!(result.accepted);
    assert_eq!(result.attempts.len(), 2);
    assert_eq!(result.attempts[0].http_status, Some(503));
    assert_eq!(result.attempts[1].http_status, Some(200));
    assert!(result.attempts[0].backoff_ms >= 1);
    assert_eq!(result.final_state, "accepted");
    assert_eq!(state.requests.load(Ordering::SeqCst), 2);

    server.abort();
}

async fn mock_relay(State(state): State<MockRelayState>) -> (StatusCode, Json<serde_json::Value>) {
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
            "relay": "mock"
        })),
    )
}
