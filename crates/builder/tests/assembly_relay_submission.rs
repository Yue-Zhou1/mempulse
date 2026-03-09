use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Router, serve};
use builder::{
    AssemblyCandidate, AssemblyCandidateKind, AssemblyConfig, AssemblyEngine, RelayBuildContext,
    RelayClient, RelayClientConfig, SimulationApproval,
};
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
async fn assembly_submits_current_block_via_relay_client() {
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
        max_retries: 0,
        initial_backoff_ms: 1,
        request_timeout_ms: 2_000,
    })
    .expect("client");
    let mut engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });
    let _ = engine.insert(candidate(
        "cand-a",
        vec![hash(0x11)],
        120,
        21_000,
        AssemblyCandidateKind::Transaction,
    ));

    let result = engine
        .submit_relay_dry_run(
            &RelayBuildContext {
                slot: 7,
                parent_hash: "0xabc".to_owned(),
                builder_pubkey: "0xdef".to_owned(),
            },
            &client,
        )
        .await
        .expect("submission should succeed")
        .expect("assembled block should produce template");

    assert!(result.accepted);
    assert_eq!(result.final_state, "accepted");
    assert_eq!(state.requests.load(Ordering::SeqCst), 1);

    server.abort();
}

#[tokio::test]
async fn assembly_skips_relay_submission_when_no_candidate_block_exists() {
    let client = RelayClient::new(RelayClientConfig {
        relay_url: "http://127.0.0.1:9/relay/v1/builder/blocks".to_owned(),
        max_retries: 0,
        initial_backoff_ms: 1,
        request_timeout_ms: 50,
    })
    .expect("client");
    let engine = AssemblyEngine::new(AssemblyConfig {
        block_gas_limit: 30_000_000,
    });

    let result = engine
        .submit_relay_dry_run(
            &RelayBuildContext {
                slot: 7,
                parent_hash: "0xabc".to_owned(),
                builder_pubkey: "0xdef".to_owned(),
            },
            &client,
        )
        .await
        .expect("empty assembly should not error");

    assert!(result.is_none());
}

fn candidate(
    candidate_id: &str,
    tx_hashes: Vec<[u8; 32]>,
    priority_score: u32,
    gas_used: u64,
    kind: AssemblyCandidateKind,
) -> AssemblyCandidate {
    AssemblyCandidate {
        candidate_id: candidate_id.to_owned(),
        tx_hashes,
        priority_score,
        gas_used,
        kind,
        simulation: SimulationApproval {
            sim_id: format!("sim-{candidate_id}"),
            block_number: 22_222_222,
            approved: true,
        },
    }
}

async fn mock_relay(State(state): State<MockRelayState>) -> (StatusCode, Json<serde_json::Value>) {
    state.requests.fetch_add(1, Ordering::SeqCst);
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "relay": "mock"
        })),
    )
}

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}
