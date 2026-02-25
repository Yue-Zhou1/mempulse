use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use builder::RelayDryRunStatus;
use common::{AlertThresholdConfig, SourceId};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxFetched, TxSeen};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use storage::{EventStore, InMemoryStorage, TxFeaturesRecord, TxFullRecord, TxSeenRecord};
use tower::util::ServiceExt;
use viz_api::auth::{ApiAuthConfig, ApiRateLimiter};
use viz_api::{
    AppState, DashboardSnapshot, InMemoryVizProvider, PropagationEdge, VizDataProvider,
    build_router,
};

const ENV_ARTIFACT_PATH: &str = "VIZ_API_TX_PERF_ARTIFACT";
const ENV_SEED_TX_COUNT: &str = "VIZ_API_TX_PERF_TX_COUNT";
const DEFAULT_ARTIFACT_PATH: &str = "artifacts/perf/tx_pipeline_perf_baseline.json";
const DEFAULT_SEED_TX_COUNT: usize = 2_000;
const STREAM_BATCH_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug)]
struct SeedSummary {
    seeded_transactions: usize,
    seeded_events: usize,
    latest_seq_id: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct TxPipelinePerfBaseline {
    seeded_transactions: usize,
    seeded_events: usize,
    latest_seq_id: u64,
    snapshot_latency_ms: f64,
    stream_catch_up_ms: f64,
    stream_events_seen: usize,
    stream_events_per_sec: f64,
}

#[tokio::test]
async fn tx_pipeline_perf_baseline_emits_metrics_artifact() {
    let seeded_transactions = std::env::var(ENV_SEED_TX_COUNT)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_SEED_TX_COUNT)
        .max(1);
    let artifact_path = artifact_path_from_env();
    let (state, seed_summary) = build_seeded_state(seeded_transactions);

    let (snapshot_latency_ms, snapshot) = measure_snapshot_latency_ms(&state).await;
    assert_eq!(snapshot.latest_seq_id, seed_summary.latest_seq_id);
    assert!(!snapshot.transactions.is_empty());

    let (stream_catch_up_ms, stream_events_seen) =
        measure_stream_catch_up(state.provider.as_ref(), seed_summary.latest_seq_id);
    assert!(stream_events_seen >= seed_summary.seeded_events);

    let stream_events_per_sec = if stream_catch_up_ms <= f64::EPSILON {
        stream_events_seen as f64
    } else {
        (stream_events_seen as f64 * 1_000.0) / stream_catch_up_ms
    };

    let baseline = TxPipelinePerfBaseline {
        seeded_transactions: seed_summary.seeded_transactions,
        seeded_events: seed_summary.seeded_events,
        latest_seq_id: seed_summary.latest_seq_id,
        snapshot_latency_ms,
        stream_catch_up_ms,
        stream_events_seen,
        stream_events_per_sec,
    };
    let baseline_stdout =
        serde_json::to_string(&baseline).expect("serialize baseline metrics for stdout");
    println!("TX_PIPELINE_PERF_BASELINE={baseline_stdout}");

    write_artifact(&artifact_path, &baseline);
    let artifact_contents =
        fs::read_to_string(&artifact_path).expect("read tx pipeline baseline artifact");
    let persisted: TxPipelinePerfBaseline =
        serde_json::from_str(&artifact_contents).expect("decode baseline artifact json");

    assert_eq!(persisted.seeded_transactions, baseline.seeded_transactions);
    assert_eq!(persisted.seeded_events, baseline.seeded_events);
    assert_eq!(persisted.latest_seq_id, baseline.latest_seq_id);
    assert_eq!(persisted.stream_events_seen, baseline.stream_events_seen);
    assert!(
        (persisted.snapshot_latency_ms - baseline.snapshot_latency_ms).abs() <= 1e-6,
        "snapshot latency mismatch persisted={} baseline={}",
        persisted.snapshot_latency_ms,
        baseline.snapshot_latency_ms
    );
    assert!(
        (persisted.stream_catch_up_ms - baseline.stream_catch_up_ms).abs() <= 1e-6,
        "stream catch-up mismatch persisted={} baseline={}",
        persisted.stream_catch_up_ms,
        baseline.stream_catch_up_ms
    );
    assert!(
        (persisted.stream_events_per_sec - baseline.stream_events_per_sec).abs() <= 1e-6,
        "stream throughput mismatch persisted={} baseline={}",
        persisted.stream_events_per_sec,
        baseline.stream_events_per_sec
    );
    assert!(baseline_stdout.contains("snapshot_latency_ms"));
    assert!(baseline_stdout.contains("stream_catch_up_ms"));
}

fn build_seeded_state(seeded_transactions: usize) -> (AppState, SeedSummary) {
    let storage = Arc::new(RwLock::new(InMemoryStorage::default()));
    let seed_summary = {
        let mut guard = storage.write().expect("lock storage for perf seed");
        seed_storage(&mut guard, seeded_transactions)
    };
    let provider = Arc::new(InMemoryVizProvider::new(
        storage,
        Arc::new(vec![PropagationEdge {
            source: "perf-seed-a".to_owned(),
            destination: "perf-seed-b".to_owned(),
            p50_delay_ms: 7,
            p99_delay_ms: 21,
        }]),
        1,
    ));
    let api_auth = ApiAuthConfig::default();
    let state = AppState {
        provider,
        downsample_limit: 1_000,
        relay_dry_run_status: Arc::new(RwLock::new(RelayDryRunStatus::default())),
        alert_thresholds: AlertThresholdConfig::default(),
        api_rate_limiter: ApiRateLimiter::new(api_auth.requests_per_minute),
        api_auth,
    };

    (state, seed_summary)
}

fn seed_storage(storage: &mut InMemoryStorage, seeded_transactions: usize) -> SeedSummary {
    let source_id = SourceId::new("perf-seed");
    let base_ts = 1_730_000_000_000_i64;
    let mut next_seq_id = 1_u64;
    for index in 0..seeded_transactions {
        let tx_seed = index as u64 + 1;
        let hash = seed_hash(tx_seed);
        let sender = seed_address(tx_seed + 100);
        let to = Some(seed_address(tx_seed + 200));
        let seen_unix_ms = base_ts + index as i64;
        let raw_tx = vec![(index % 251) as u8; 64];

        storage.append_event(EventEnvelope {
            seq_id: next_seq_id,
            ingest_ts_unix_ms: seen_unix_ms,
            ingest_ts_mono_ns: next_seq_id.saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload: EventPayload::TxSeen(TxSeen {
                hash,
                peer_id: "perf-seed-peer".to_owned(),
                seen_at_unix_ms: seen_unix_ms,
                seen_at_mono_ns: next_seq_id.saturating_mul(1_000_000),
            }),
        });
        next_seq_id = next_seq_id.saturating_add(1);

        storage.append_event(EventEnvelope {
            seq_id: next_seq_id,
            ingest_ts_unix_ms: seen_unix_ms + 1,
            ingest_ts_mono_ns: next_seq_id.saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload: EventPayload::TxFetched(TxFetched {
                hash,
                fetched_at_unix_ms: seen_unix_ms + 1,
            }),
        });
        next_seq_id = next_seq_id.saturating_add(1);

        storage.append_event(EventEnvelope {
            seq_id: next_seq_id,
            ingest_ts_unix_ms: seen_unix_ms + 2,
            ingest_ts_mono_ns: next_seq_id.saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload: EventPayload::TxDecoded(TxDecoded {
                hash,
                tx_type: 2,
                sender,
                nonce: tx_seed,
                chain_id: Some(1),
                to,
                value_wei: Some(1_000_000_000_000_000),
                gas_limit: Some(21000),
                gas_price_wei: Some(20_000_000_000),
                max_fee_per_gas_wei: Some(30_000_000_000),
                max_priority_fee_per_gas_wei: Some(2_000_000_000),
                max_fee_per_blob_gas_wei: None,
                calldata_len: Some(raw_tx.len() as u32),
            }),
        });
        next_seq_id = next_seq_id.saturating_add(1);

        storage.upsert_tx_seen(TxSeenRecord {
            hash,
            peer: "perf-seed-peer".to_owned(),
            first_seen_unix_ms: seen_unix_ms,
            first_seen_mono_ns: tx_seed.saturating_mul(1_000_000),
            seen_count: 1,
        });
        storage.upsert_tx_full(TxFullRecord {
            hash,
            tx_type: 2,
            sender,
            nonce: tx_seed,
            to,
            chain_id: Some(1),
            value_wei: Some(1_000_000_000_000_000),
            gas_limit: Some(21000),
            gas_price_wei: Some(20_000_000_000),
            max_fee_per_gas_wei: Some(30_000_000_000),
            max_priority_fee_per_gas_wei: Some(2_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(raw_tx.len() as u32),
            raw_tx: raw_tx.clone(),
        });
        storage.upsert_tx_features(TxFeaturesRecord {
            hash,
            chain_id: Some(1),
            protocol: "perf-protocol".to_owned(),
            category: "swap".to_owned(),
            mev_score: (10 + (index % 80)) as u16,
            urgency_score: (5 + (index % 50)) as u16,
            method_selector: Some([0xde, 0xad, 0xbe, 0xef]),
            feature_engine_version: "feature-engine.perf-baseline".to_owned(),
        });
    }

    let seeded_events = seeded_transactions.saturating_mul(3);
    SeedSummary {
        seeded_transactions,
        seeded_events,
        latest_seq_id: next_seq_id.saturating_sub(1),
    }
}

fn seed_hash(seed: u64) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    hash[..8].copy_from_slice(&seed.to_be_bytes());
    hash[8..16].copy_from_slice(&(seed.saturating_mul(17)).to_be_bytes());
    hash
}

fn seed_address(seed: u64) -> [u8; 20] {
    let mut address = [0_u8; 20];
    address[..8].copy_from_slice(&seed.to_be_bytes());
    address[8..16].copy_from_slice(&(seed.saturating_mul(11)).to_be_bytes());
    address[16..20].copy_from_slice(&(seed as u32).to_be_bytes());
    address
}

async fn measure_snapshot_latency_ms(state: &AppState) -> (f64, DashboardSnapshot) {
    let app = build_router(state.clone());
    let start = Instant::now();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/snapshot?tx_limit=500&feature_limit=500&opp_limit=500&replay_limit=500")
                .body(Body::empty())
                .expect("build snapshot request"),
        )
        .await
        .expect("execute snapshot request");
    let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("read snapshot response body");
    let snapshot: DashboardSnapshot =
        serde_json::from_slice(&body).expect("decode dashboard snapshot json");
    (elapsed_ms, snapshot)
}

fn measure_stream_catch_up(provider: &dyn VizDataProvider, latest_seq_id: u64) -> (f64, usize) {
    let start = Instant::now();
    let mut after_seq_id = 0_u64;
    let mut streamed_events = 0_usize;
    loop {
        let batch = provider.events(after_seq_id, &[], STREAM_BATCH_LIMIT);
        if batch.is_empty() {
            break;
        }

        let mut progressed = false;
        for event in batch {
            if event.seq_id <= after_seq_id {
                continue;
            }
            progressed = true;
            after_seq_id = event.seq_id;
            streamed_events = streamed_events.saturating_add(1);
            if after_seq_id >= latest_seq_id {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
                return (elapsed_ms, streamed_events);
            }
        }

        if !progressed {
            break;
        }
    }

    (start.elapsed().as_secs_f64() * 1_000.0, streamed_events)
}

fn artifact_path_from_env() -> PathBuf {
    let candidate = std::env::var(ENV_ARTIFACT_PATH)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ARTIFACT_PATH));
    if candidate.is_absolute() {
        candidate
    } else {
        workspace_root().join(candidate)
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve workspace root")
}

fn write_artifact(path: &Path, baseline: &TxPipelinePerfBaseline) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create tx pipeline perf artifact directory");
    }
    let payload =
        serde_json::to_string_pretty(baseline).expect("serialize tx pipeline perf baseline");
    fs::write(path, payload).expect("write tx pipeline perf baseline artifact");
}
