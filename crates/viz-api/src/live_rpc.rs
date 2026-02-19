use anyhow::{Context, Result, anyhow};
use common::{SourceId, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxFetched, TxSeen};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::env;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{EventStore, InMemoryStorage, TxFeaturesRecord, TxFullRecord, TxSeenRecord};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Debug)]
pub struct LiveRpcConfig {
    ws_url: String,
    http_url: Option<String>,
    source_id: SourceId,
    max_seen_hashes: usize,
}

impl LiveRpcConfig {
    pub fn from_env() -> Option<Self> {
        let ws_url = env::var("VIZ_API_ETH_WS_URL").ok()?.trim().to_owned();
        if ws_url.is_empty() {
            return None;
        }
        let http_url = env::var("VIZ_API_ETH_HTTP_URL").ok().and_then(|url| {
            let trimmed = url.trim().to_owned();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        let source_id = SourceId::new(
            env::var("VIZ_API_SOURCE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "rpc-live".to_owned()),
        );
        let max_seen_hashes = env::var("VIZ_API_MAX_SEEN_HASHES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10_000)
            .max(100);

        Some(Self {
            ws_url,
            http_url,
            source_id,
            max_seen_hashes,
        })
    }
}

pub fn start_live_rpc_feed(storage: Arc<RwLock<InMemoryStorage>>, config: LiveRpcConfig) {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => return,
    };

    handle.spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .build()
            .expect("reqwest client");

        let mut seen_hashes = HashSet::new();
        let mut seen_order = VecDeque::new();
        let mut next_seq_id = current_seq_hi(&storage).saturating_add(1).max(1);

        loop {
            let session_result = run_ws_session(
                &storage,
                &config,
                &client,
                &mut next_seq_id,
                &mut seen_hashes,
                &mut seen_order,
            )
            .await;

            if let Err(err) = session_result {
                tracing::warn!(error = %err, "live rpc websocket session ended");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn run_ws_session(
    storage: &Arc<RwLock<InMemoryStorage>>,
    config: &LiveRpcConfig,
    client: &reqwest::Client,
    next_seq_id: &mut u64,
    seen_hashes: &mut HashSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(config.ws_url.as_str())
        .await
        .with_context(|| format!("connect {}", config.ws_url))?;
    let (mut write, mut read) = ws_stream.split();

    let subscribe = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["newPendingTransactions"],
    });
    write
        .send(Message::Text(subscribe.to_string().into()))
        .await
        .context("send eth_subscribe request")?;

    let mut subscription_id: Option<String> = None;
    while let Some(frame) = read.next().await {
        let frame = frame.context("read websocket frame")?;
        match frame {
            Message::Text(text) => {
                if let Some(hash_hex) = parse_pending_hash(&text, &mut subscription_id) {
                    process_pending_hash(
                        storage,
                        config,
                        client,
                        &hash_hex,
                        next_seq_id,
                        seen_hashes,
                        seen_order,
                    )
                    .await?;
                }
            }
            Message::Ping(payload) => {
                if write.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

fn parse_pending_hash(payload: &str, subscription_id: &mut Option<String>) -> Option<String> {
    let value: Value = serde_json::from_str(payload).ok()?;
    if value.get("id").is_some() {
        if let Some(result) = value.get("result").and_then(Value::as_str) {
            *subscription_id = Some(result.to_owned());
        }
        return None;
    }
    if value.get("method").and_then(Value::as_str) != Some("eth_subscription") {
        return None;
    }

    let params = value.get("params")?;
    let incoming_sub = params.get("subscription").and_then(Value::as_str)?;
    if let Some(expected) = subscription_id.as_ref() {
        if expected != incoming_sub {
            return None;
        }
    }
    let result = params.get("result")?;
    if let Some(hash) = result.as_str() {
        return Some(hash.to_owned());
    }
    result
        .get("hash")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

async fn process_pending_hash(
    storage: &Arc<RwLock<InMemoryStorage>>,
    config: &LiveRpcConfig,
    client: &reqwest::Client,
    hash_hex: &str,
    next_seq_id: &mut u64,
    seen_hashes: &mut HashSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
) -> Result<()> {
    let hash = match parse_fixed_hex::<32>(hash_hex) {
        Some(hash) => hash,
        None => return Ok(()),
    };
    if !remember_hash(hash, seen_hashes, seen_order, config.max_seen_hashes) {
        return Ok(());
    }

    let now_unix_ms = current_unix_ms();
    let mut fetched_tx = None;
    if let Some(http_url) = config.http_url.as_deref() {
        match fetch_transaction_by_hash(client, http_url, hash_hex).await {
            Ok(tx) => fetched_tx = tx,
            Err(err) => tracing::warn!(error = %err, hash = hash_hex, "fetch tx by hash failed"),
        }
    }

    let mut store = storage
        .write()
        .map_err(|_| anyhow!("storage lock poisoned during live rpc ingest"))?;

    append_event(
        &mut store,
        next_seq_id,
        &config.source_id,
        now_unix_ms,
        EventPayload::TxSeen(TxSeen {
            hash,
            peer_id: "rpc-ws".to_owned(),
            seen_at_unix_ms: now_unix_ms,
            seen_at_mono_ns: next_seq_id.saturating_mul(1_000_000),
        }),
    );
    store.upsert_tx_seen(TxSeenRecord {
        hash,
        peer: "rpc-ws".to_owned(),
        first_seen_unix_ms: now_unix_ms,
        first_seen_mono_ns: next_seq_id.saturating_mul(1_000_000),
        seen_count: 1,
    });

    append_event(
        &mut store,
        next_seq_id,
        &config.source_id,
        now_unix_ms,
        EventPayload::TxFetched(TxFetched {
            hash,
            fetched_at_unix_ms: now_unix_ms,
        }),
    );

    if let Some(tx) = fetched_tx {
        append_event(
            &mut store,
            next_seq_id,
            &config.source_id,
            now_unix_ms,
            EventPayload::TxDecoded(TxDecoded {
                hash: tx.hash,
                tx_type: tx.tx_type,
                sender: tx.sender,
                nonce: tx.nonce,
            }),
        );
        store.upsert_tx_full(TxFullRecord {
            hash: tx.hash,
            sender: tx.sender,
            nonce: tx.nonce,
            raw_tx: tx.input,
        });
        store.upsert_tx_features(TxFeaturesRecord {
            hash: tx.hash,
            protocol: "unknown".to_owned(),
            category: "pending".to_owned(),
            mev_score: 0,
        });
    }

    Ok(())
}

fn append_event(
    store: &mut InMemoryStorage,
    next_seq_id: &mut u64,
    source_id: &SourceId,
    now_unix_ms: i64,
    payload: EventPayload,
) {
    let seq_id = *next_seq_id;
    *next_seq_id = next_seq_id.saturating_add(1);
    store.append_event(EventEnvelope {
        seq_id,
        ingest_ts_unix_ms: now_unix_ms,
        ingest_ts_mono_ns: seq_id.saturating_mul(1_000_000),
        source_id: source_id.clone(),
        payload,
    });
}

fn remember_hash(
    hash: TxHash,
    seen_hashes: &mut HashSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
    max_seen_hashes: usize,
) -> bool {
    if !seen_hashes.insert(hash) {
        return false;
    }
    seen_order.push_back(hash);
    while seen_order.len() > max_seen_hashes {
        if let Some(old_hash) = seen_order.pop_front() {
            seen_hashes.remove(&old_hash);
        }
    }
    true
}

#[derive(Debug, Deserialize)]
struct RpcTransaction {
    hash: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default, rename = "type")]
    tx_type: Option<String>,
    #[serde(default)]
    input: Option<String>,
}

#[derive(Clone, Debug)]
struct LiveTx {
    hash: TxHash,
    sender: [u8; 20],
    nonce: u64,
    tx_type: u8,
    input: Vec<u8>,
}

async fn fetch_transaction_by_hash(
    client: &reqwest::Client,
    http_url: &str,
    hash_hex: &str,
) -> Result<Option<LiveTx>> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "eth_getTransactionByHash",
        "params": [hash_hex],
    });

    let response: Value = client
        .post(http_url)
        .json(&request_body)
        .send()
        .await
        .with_context(|| format!("POST {http_url}"))?
        .error_for_status()
        .context("rpc http status error")?
        .json()
        .await
        .context("decode rpc json response")?;

    if let Some(error_value) = response.get("error") {
        return Err(anyhow!("rpc returned error: {error_value}"));
    }
    let result = match response.get("result") {
        Some(value) if !value.is_null() => value.clone(),
        _ => return Ok(None),
    };
    let tx: RpcTransaction = serde_json::from_value(result).context("decode rpc transaction")?;

    let hash = parse_fixed_hex::<32>(&tx.hash)
        .or_else(|| parse_fixed_hex::<32>(hash_hex))
        .ok_or_else(|| anyhow!("invalid transaction hash"))?;
    let sender = tx
        .from
        .as_deref()
        .and_then(parse_fixed_hex::<20>)
        .unwrap_or([0_u8; 20]);
    let nonce = tx
        .nonce
        .as_deref()
        .and_then(parse_hex_u64)
        .unwrap_or_default();
    let tx_type = tx
        .tx_type
        .as_deref()
        .and_then(parse_hex_u64)
        .unwrap_or_default() as u8;
    let input = tx
        .input
        .as_deref()
        .and_then(parse_hex_bytes)
        .unwrap_or_default();

    Ok(Some(LiveTx {
        hash,
        sender,
        nonce,
        tx_type,
        input,
    }))
}

fn parse_fixed_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let bytes = parse_hex_bytes(value)?;
    if bytes.len() != N {
        return None;
    }
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn parse_hex_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    if trimmed.len() % 2 != 0 {
        return None;
    }
    (0..trimmed.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&trimmed[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(trimmed, 16).ok()
}

fn current_seq_hi(storage: &Arc<RwLock<InMemoryStorage>>) -> u64 {
    storage
        .read()
        .ok()
        .and_then(|store| store.list_events().last().map(|event| event.seq_id))
        .unwrap_or(0)
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
