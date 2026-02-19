use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub replay: Arc<Vec<ReplayPoint>>,
    pub propagation: Arc<Vec<PropagationEdge>>,
    pub features: Arc<Vec<FeatureSummary>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayPoint {
    pub seq_hi: u64,
    pub timestamp_unix_ms: i64,
    pub pending_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PropagationEdge {
    pub source: String,
    pub destination: String,
    pub p50_delay_ms: u32,
    pub p99_delay_ms: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeatureSummary {
    pub protocol: String,
    pub category: String,
    pub count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamHello {
    pub event: String,
    pub message: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/replay", get(replay))
        .route("/propagation", get(propagation))
        .route("/features", get(features))
        .route("/stream", get(stream))
        .with_state(state)
}

pub fn default_state() -> AppState {
    AppState {
        replay: Arc::new(vec![
            ReplayPoint {
                seq_hi: 1_000,
                timestamp_unix_ms: 1_700_000_000_000,
                pending_count: 2_000,
            },
            ReplayPoint {
                seq_hi: 1_200,
                timestamp_unix_ms: 1_700_000_012_000,
                pending_count: 2_200,
            },
        ]),
        propagation: Arc::new(vec![
            PropagationEdge {
                source: "peer-a".to_owned(),
                destination: "peer-b".to_owned(),
                p50_delay_ms: 8,
                p99_delay_ms: 24,
            },
            PropagationEdge {
                source: "peer-a".to_owned(),
                destination: "peer-c".to_owned(),
                p50_delay_ms: 12,
                p99_delay_ms: 36,
            },
        ]),
        features: Arc::new(vec![
            FeatureSummary {
                protocol: "uniswap-v2".to_owned(),
                category: "swap".to_owned(),
                count: 120,
            },
            FeatureSummary {
                protocol: "aave".to_owned(),
                category: "borrow".to_owned(),
                count: 35,
            },
        ]),
    }
}

async fn health() -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_owned(),
        }),
    )
}

async fn replay(State(state): State<AppState>) -> Json<Vec<ReplayPoint>> {
    Json((*state.replay).clone())
}

async fn propagation(State(state): State<AppState>) -> Json<Vec<PropagationEdge>> {
    Json((*state.propagation).clone())
}

async fn features(State(state): State<AppState>) -> Json<Vec<FeatureSummary>> {
    Json((*state.features).clone())
}

async fn stream(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    let hello = StreamHello {
        event: "hello".to_owned(),
        message: "viz-api stream connected".to_owned(),
    };

    if let Ok(payload) = serde_json::to_string(&hello) {
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }

    while let Some(Ok(message)) = socket.recv().await {
        match message {
            Message::Close(_) => break,
            Message::Ping(data) => {
                if socket.send(Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn health_route_returns_ok() {
        let app = build_router(default_state());

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let payload: HealthResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.status, "ok");
    }

    #[tokio::test]
    async fn replay_route_returns_seed_data() {
        let app = build_router(default_state());

        let response = app
            .oneshot(Request::builder().uri("/replay").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Vec<ReplayPoint> = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0].seq_hi, 1_000);
    }
}
