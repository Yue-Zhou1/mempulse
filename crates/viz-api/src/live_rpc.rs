use ahash::RandomState;
use anyhow::{Context, Result, anyhow};
use common::{SourceId, TxHash};
use event_log::{
    BundleSubmitted, EventEnvelope, EventPayload, OppDetected, SimCompleted, TxDecoded, TxFetched,
    TxSeen,
};
use feature_engine::{
    FeatureAnalysis, FeatureInput, analyze_transaction, version as feature_engine_version,
};
use futures::{SinkExt, StreamExt};
use hashbrown::HashSet;
use searcher::{SearcherConfig, SearcherInputTx, rank_opportunities};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{
    EventStore, InMemoryStorage, OpportunityRecord, StorageWriteHandle, StorageWriteOp,
    TxFeaturesRecord, TxFullRecord, TxSeenRecord,
};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type FastSet<T> = HashSet<T, RandomState>;

const PRIMARY_PUBLIC_WS_URL: &str = "wss://eth.drpc.org";
const PRIMARY_PUBLIC_HTTP_URL: &str = "https://eth.drpc.org";
const FALLBACK_PUBLIC_WS_URL: &str = "wss://ethereum-rpc.publicnode.com";
const FALLBACK_PUBLIC_HTTP_URL: &str = "https://ethereum-rpc.publicnode.com";
const ENV_ETH_WS_URL: &str = "VIZ_API_ETH_WS_URL";
const ENV_ETH_HTTP_URL: &str = "VIZ_API_ETH_HTTP_URL";
const ENV_SOURCE_ID: &str = "VIZ_API_SOURCE_ID";
const ENV_MAX_SEEN_HASHES: &str = "VIZ_API_MAX_SEEN_HASHES";
const SEARCHER_MIN_SCORE: u32 = 0;
const SEARCHER_MAX_CANDIDATES: usize = 8;

#[derive(Clone, Debug)]
struct RpcEndpoint {
    ws_url: &'static str,
    http_url: &'static str,
}

#[derive(Clone, Debug)]
pub struct LiveRpcConfig {
    endpoints: Vec<RpcEndpoint>,
    source_id: SourceId,
    max_seen_hashes: usize,
}

impl Default for LiveRpcConfig {
    fn default() -> Self {
        Self {
            endpoints: vec![
                RpcEndpoint {
                    ws_url: PRIMARY_PUBLIC_WS_URL,
                    http_url: PRIMARY_PUBLIC_HTTP_URL,
                },
                RpcEndpoint {
                    ws_url: FALLBACK_PUBLIC_WS_URL,
                    http_url: FALLBACK_PUBLIC_HTTP_URL,
                },
            ],
            source_id: SourceId::new("rpc-live"),
            max_seen_hashes: 10_000,
        }
    }
}

impl LiveRpcConfig {
    pub fn from_env() -> Result<Self> {
        let ws_override = std::env::var(ENV_ETH_WS_URL)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let http_override = std::env::var(ENV_ETH_HTTP_URL)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let source_id = std::env::var(ENV_SOURCE_ID)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "rpc-live".to_owned());
        let max_seen_hashes = std::env::var(ENV_MAX_SEEN_HASHES)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| value.parse::<usize>())
            .transpose()
            .map_err(|err| anyhow!("invalid {ENV_MAX_SEEN_HASHES}: {err}"))?
            .unwrap_or(10_000)
            .max(1);

        let mut config = Self::default();
        if let Some(ws_url) = ws_override {
            let ws_url_static: &'static str = Box::leak(ws_url.into_boxed_str());
            let http_url_static: &'static str = Box::leak(
                http_override
                    .unwrap_or_else(|| PRIMARY_PUBLIC_HTTP_URL.to_owned())
                    .into_boxed_str(),
            );
            config.endpoints = vec![RpcEndpoint {
                ws_url: ws_url_static,
                http_url: http_url_static,
            }];
        } else if let Some(http_url) = http_override {
            let http_url_static: &'static str = Box::leak(http_url.into_boxed_str());
            if let Some(primary) = config.endpoints.first_mut() {
                primary.http_url = http_url_static;
            }
        }

        config.source_id = SourceId::new(source_id);
        config.max_seen_hashes = max_seen_hashes;
        Ok(config)
    }

    pub fn primary_ws_url(&self) -> Option<&str> {
        self.endpoints.first().map(|endpoint| endpoint.ws_url)
    }

    pub fn primary_http_url(&self) -> Option<&str> {
        self.endpoints.first().map(|endpoint| endpoint.http_url)
    }

    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    pub fn max_seen_hashes(&self) -> usize {
        self.max_seen_hashes
    }
}

pub fn start_live_rpc_feed(
    storage: Arc<RwLock<InMemoryStorage>>,
    writer: StorageWriteHandle,
    config: LiveRpcConfig,
) {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => return,
    };

    handle.spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .build()
            .expect("reqwest client");
        if config.endpoints.is_empty() {
            tracing::error!("live rpc config has no endpoints");
            return;
        }

        let mut seen_hashes = FastSet::default();
        let mut seen_order = VecDeque::new();
        let mut next_seq_id = current_seq_hi(&storage).saturating_add(1).max(1);
        let mut endpoint_index = 0usize;

        loop {
            let endpoint = &config.endpoints[endpoint_index];
            tracing::info!(
                ws_url = endpoint.ws_url,
                http_url = endpoint.http_url,
                endpoint_index,
                "starting live rpc websocket session"
            );
            let session_result = run_ws_session(
                &writer,
                endpoint,
                &config.source_id,
                config.max_seen_hashes,
                &client,
                &mut next_seq_id,
                &mut seen_hashes,
                &mut seen_order,
            )
            .await;

            if let Err(err) = session_result {
                let error_chain = format_error_chain(&err);
                tracing::warn!(
                    error = %err,
                    error_chain = %error_chain,
                    ws_url = endpoint.ws_url,
                    http_url = endpoint.http_url,
                    "live rpc websocket session ended; rotating endpoint"
                );
                endpoint_index = (endpoint_index + 1) % config.endpoints.len();
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn run_ws_session(
    writer: &StorageWriteHandle,
    endpoint: &RpcEndpoint,
    source_id: &SourceId,
    max_seen_hashes: usize,
    client: &reqwest::Client,
    next_seq_id: &mut u64,
    seen_hashes: &mut FastSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(endpoint.ws_url)
        .await
        .with_context(|| format!("connect {}", endpoint.ws_url))?;
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
    tracing::info!(
        ws_url = endpoint.ws_url,
        "subscribed to eth_subscribe:newPendingTransactions"
    );

    let mut subscription_id: Option<String> = None;
    while let Some(frame) = read.next().await {
        let frame = frame.context("read websocket frame")?;
        match frame {
            Message::Text(text) => {
                if let Some(hash_hex) = parse_pending_hash(&text, &mut subscription_id) {
                    process_pending_hash(
                        writer,
                        endpoint,
                        source_id,
                        max_seen_hashes,
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
    writer: &StorageWriteHandle,
    endpoint: &RpcEndpoint,
    source_id: &SourceId,
    max_seen_hashes: usize,
    client: &reqwest::Client,
    hash_hex: &str,
    next_seq_id: &mut u64,
    seen_hashes: &mut FastSet<TxHash>,
    seen_order: &mut VecDeque<TxHash>,
) -> Result<()> {
    let hash = match parse_fixed_hex::<32>(hash_hex) {
        Some(hash) => hash,
        None => return Ok(()),
    };
    if !remember_hash(hash, seen_hashes, seen_order, max_seen_hashes) {
        return Ok(());
    }

    let now_unix_ms = current_unix_ms();
    let fetched_tx = match fetch_transaction_by_hash(client, endpoint.http_url, hash_hex).await {
        Ok(tx) => tx,
        Err(err) => {
            let error_chain = format_error_chain(&err);
            tracing::warn!(
                error = %err,
                error_chain = %error_chain,
                hash = hash_hex,
                http_url = endpoint.http_url,
                "fetch tx by hash failed"
            );
            None
        }
    };

    let feature_analysis = fetched_tx
        .as_ref()
        .map(|tx| analyze_transaction(feature_input(tx)));

    if let Some(tx) = fetched_tx.as_ref() {
        let to = format_optional_fixed_hex(tx.to.as_ref().map(|value| value.as_slice()));
        let chain_id = format_optional_u64(tx.chain_id);
        let gas_limit = format_optional_u64(tx.gas_limit);
        let value_wei = format_optional_u128(tx.value_wei);
        let gas_price_wei = format_optional_u128(tx.gas_price_wei);
        let max_fee_per_gas_wei = format_optional_u128(tx.max_fee_per_gas_wei);
        let max_priority_fee_per_gas_wei = format_optional_u128(tx.max_priority_fee_per_gas_wei);
        let max_fee_per_blob_gas_wei = format_optional_u128(tx.max_fee_per_blob_gas_wei);
        let analysis = feature_analysis.unwrap_or(default_feature_analysis());
        let method_selector = format_method_selector_hex(analysis.method_selector);
        tracing::info!(
            hash = hash_hex,
            sender = %format_fixed_hex(&tx.sender),
            to = %to,
            nonce = tx.nonce,
            tx_type = tx.tx_type,
            chain_id = %chain_id,
            gas_limit = %gas_limit,
            value_wei = %value_wei,
            gas_price_wei = %gas_price_wei,
            max_fee_per_gas_wei = %max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei = %max_priority_fee_per_gas_wei,
            max_fee_per_blob_gas_wei = %max_fee_per_blob_gas_wei,
            protocol = analysis.protocol,
            category = analysis.category,
            mev_score = analysis.mev_score,
            urgency_score = analysis.urgency_score,
            method_selector = %method_selector,
            input_bytes = tx.input.len(),
            "mempool transaction"
        );
    } else {
        tracing::info!(
            hash = hash_hex,
            "mempool transaction (details unavailable from rpc)"
        );
    }

    append_event(
        writer,
        next_seq_id,
        source_id,
        now_unix_ms,
        EventPayload::TxSeen(TxSeen {
            hash,
            peer_id: "rpc-ws".to_owned(),
            seen_at_unix_ms: now_unix_ms,
            seen_at_mono_ns: next_seq_id.saturating_mul(1_000_000),
        }),
    )
    .await?;
    writer
        .enqueue(StorageWriteOp::UpsertTxSeen(TxSeenRecord {
            hash,
            peer: "rpc-ws".to_owned(),
            first_seen_unix_ms: now_unix_ms,
            first_seen_mono_ns: next_seq_id.saturating_mul(1_000_000),
            seen_count: 1,
        }))
        .await?;

    append_event(
        writer,
        next_seq_id,
        source_id,
        now_unix_ms,
        EventPayload::TxFetched(TxFetched {
            hash,
            fetched_at_unix_ms: now_unix_ms,
        }),
    )
    .await?;

    if let Some(tx) = fetched_tx {
        let analysis = feature_analysis.unwrap_or(default_feature_analysis());
        let raw_tx = tx.input.clone();
        let decoded = TxDecoded {
            hash: tx.hash,
            tx_type: tx.tx_type,
            sender: tx.sender,
            nonce: tx.nonce,
            chain_id: tx.chain_id,
            to: tx.to,
            value_wei: tx.value_wei,
            gas_limit: tx.gas_limit,
            gas_price_wei: tx.gas_price_wei,
            max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
            max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
            calldata_len: Some(raw_tx.len() as u32),
        };
        append_event(
            writer,
            next_seq_id,
            source_id,
            now_unix_ms,
            EventPayload::TxDecoded(decoded.clone()),
        )
        .await?;
        writer
            .enqueue(StorageWriteOp::UpsertTxFull(TxFullRecord {
                hash: tx.hash,
                tx_type: tx.tx_type,
                sender: tx.sender,
                nonce: tx.nonce,
                to: tx.to,
                chain_id: tx.chain_id,
                value_wei: tx.value_wei,
                gas_limit: tx.gas_limit,
                gas_price_wei: tx.gas_price_wei,
                max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
                max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
                max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
                calldata_len: Some(raw_tx.len() as u32),
                raw_tx: raw_tx.clone(),
            }))
            .await?;
        writer
            .enqueue(StorageWriteOp::UpsertTxFeatures(TxFeaturesRecord {
                hash: tx.hash,
                protocol: analysis.protocol.to_owned(),
                category: analysis.category.to_owned(),
                mev_score: analysis.mev_score,
                urgency_score: analysis.urgency_score,
                method_selector: analysis.method_selector,
                feature_engine_version: feature_engine_version().to_owned(),
            }))
            .await?;
        for opportunity in build_opportunity_records(&decoded, &raw_tx, now_unix_ms) {
            let events =
                build_rule_versioned_events(*next_seq_id, now_unix_ms, source_id, &opportunity);
            *next_seq_id = next_seq_id.saturating_add(events.len() as u64);
            for event in events {
                writer.enqueue(StorageWriteOp::AppendEvent(event)).await?;
            }
            writer
                .enqueue(StorageWriteOp::UpsertOpportunity(opportunity))
                .await?;
        }
    }

    Ok(())
}

async fn append_event(
    writer: &StorageWriteHandle,
    next_seq_id: &mut u64,
    source_id: &SourceId,
    now_unix_ms: i64,
    payload: EventPayload,
) -> Result<()> {
    let seq_id = *next_seq_id;
    *next_seq_id = next_seq_id.saturating_add(1);
    writer
        .enqueue(StorageWriteOp::AppendEvent(EventEnvelope {
            seq_id,
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: seq_id.saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload,
        }))
        .await?;
    Ok(())
}

fn remember_hash(
    hash: TxHash,
    seen_hashes: &mut FastSet<TxHash>,
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
    to: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    gas: Option<String>,
    #[serde(default, rename = "type")]
    tx_type: Option<String>,
    #[serde(default, rename = "chainId")]
    chain_id: Option<String>,
    #[serde(default, rename = "gasPrice")]
    gas_price: Option<String>,
    #[serde(default, rename = "maxFeePerGas")]
    max_fee_per_gas: Option<String>,
    #[serde(default, rename = "maxPriorityFeePerGas")]
    max_priority_fee_per_gas: Option<String>,
    #[serde(default, rename = "maxFeePerBlobGas")]
    max_fee_per_blob_gas: Option<String>,
    #[serde(default)]
    input: Option<String>,
}

#[derive(Clone, Debug)]
struct LiveTx {
    hash: TxHash,
    sender: [u8; 20],
    to: Option<[u8; 20]>,
    nonce: u64,
    tx_type: u8,
    value_wei: Option<u128>,
    gas_limit: Option<u64>,
    chain_id: Option<u64>,
    gas_price_wei: Option<u128>,
    max_fee_per_gas_wei: Option<u128>,
    max_priority_fee_per_gas_wei: Option<u128>,
    max_fee_per_blob_gas_wei: Option<u128>,
    input: Vec<u8>,
}

#[inline]
fn feature_input(tx: &LiveTx) -> FeatureInput<'_> {
    FeatureInput {
        to: tx.to.as_ref(),
        calldata: &tx.input,
        tx_type: tx.tx_type,
        chain_id: tx.chain_id,
        gas_limit: tx.gas_limit,
        value_wei: tx.value_wei,
        gas_price_wei: tx.gas_price_wei,
        max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
    }
}

#[inline]
fn default_feature_analysis() -> FeatureAnalysis {
    FeatureAnalysis {
        protocol: "unknown",
        category: "pending",
        mev_score: 0,
        urgency_score: 0,
        method_selector: None,
    }
}

fn build_opportunity_records(
    decoded: &TxDecoded,
    calldata: &[u8],
    detected_unix_ms: i64,
) -> Vec<OpportunityRecord> {
    rank_opportunities(
        &[SearcherInputTx {
            decoded: decoded.clone(),
            calldata: calldata.to_vec(),
        }],
        SearcherConfig {
            min_score: SEARCHER_MIN_SCORE,
            max_candidates: SEARCHER_MAX_CANDIDATES,
        },
    )
    .into_iter()
    .map(|candidate| OpportunityRecord {
        tx_hash: candidate.tx_hash,
        strategy: format!("{:?}", candidate.strategy),
        score: candidate.score,
        protocol: candidate.protocol,
        category: candidate.category,
        feature_engine_version: candidate.feature_engine_version,
        scorer_version: candidate.scorer_version,
        strategy_version: candidate.strategy_version,
        reasons: candidate.reasons,
        detected_unix_ms,
    })
    .collect()
}

fn build_rule_versioned_events(
    start_seq_id: u64,
    now_unix_ms: i64,
    source_id: &SourceId,
    opportunity: &OpportunityRecord,
) -> Vec<EventEnvelope> {
    let hash_prefix = format_fixed_hex(&opportunity.tx_hash)[2..10].to_owned();
    let sim_id = format!("sim-{hash_prefix}-{}", opportunity.detected_unix_ms);
    let bundle_id = format!("bundle-{hash_prefix}-{}", opportunity.detected_unix_ms);

    vec![
        EventEnvelope {
            seq_id: start_seq_id,
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: start_seq_id.saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload: EventPayload::OppDetected(OppDetected {
                hash: opportunity.tx_hash,
                strategy: opportunity.strategy.clone(),
                score: opportunity.score,
                protocol: opportunity.protocol.clone(),
                category: opportunity.category.clone(),
                feature_engine_version: opportunity.feature_engine_version.clone(),
                scorer_version: opportunity.scorer_version.clone(),
                strategy_version: opportunity.strategy_version.clone(),
                reasons: opportunity.reasons.clone(),
            }),
        },
        EventEnvelope {
            seq_id: start_seq_id.saturating_add(1),
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: start_seq_id.saturating_add(1).saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload: EventPayload::SimCompleted(SimCompleted {
                hash: opportunity.tx_hash,
                sim_id: sim_id.clone(),
                status: "not_run".to_owned(),
                feature_engine_version: opportunity.feature_engine_version.clone(),
                scorer_version: opportunity.scorer_version.clone(),
                strategy_version: opportunity.strategy_version.clone(),
            }),
        },
        EventEnvelope {
            seq_id: start_seq_id.saturating_add(2),
            ingest_ts_unix_ms: now_unix_ms,
            ingest_ts_mono_ns: start_seq_id.saturating_add(2).saturating_mul(1_000_000),
            source_id: source_id.clone(),
            payload: EventPayload::BundleSubmitted(BundleSubmitted {
                hash: opportunity.tx_hash,
                bundle_id,
                sim_id,
                relay: "not_submitted".to_owned(),
                accepted: false,
                feature_engine_version: opportunity.feature_engine_version.clone(),
                scorer_version: opportunity.scorer_version.clone(),
                strategy_version: opportunity.strategy_version.clone(),
            }),
        },
    ]
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
    Ok(Some(rpc_tx_to_live_tx(tx, hash_hex)?))
}

fn rpc_tx_to_live_tx(tx: RpcTransaction, hash_hex: &str) -> Result<LiveTx> {
    let hash = parse_fixed_hex::<32>(&tx.hash)
        .or_else(|| parse_fixed_hex::<32>(hash_hex))
        .ok_or_else(|| anyhow!("invalid transaction hash"))?;
    let sender = tx
        .from
        .as_deref()
        .and_then(parse_fixed_hex::<20>)
        .unwrap_or([0_u8; 20]);
    let to = tx.to.as_deref().and_then(parse_fixed_hex::<20>);
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
    let value_wei = tx.value.as_deref().and_then(parse_hex_u128);
    let gas_limit = tx.gas.as_deref().and_then(parse_hex_u64);
    let chain_id = tx.chain_id.as_deref().and_then(parse_hex_u64);
    let gas_price_wei = tx.gas_price.as_deref().and_then(parse_hex_u128);
    let max_fee_per_gas_wei = tx.max_fee_per_gas.as_deref().and_then(parse_hex_u128);
    let max_priority_fee_per_gas_wei = tx
        .max_priority_fee_per_gas
        .as_deref()
        .and_then(parse_hex_u128);
    let max_fee_per_blob_gas_wei = tx.max_fee_per_blob_gas.as_deref().and_then(parse_hex_u128);

    Ok(LiveTx {
        hash,
        sender,
        to,
        nonce,
        tx_type,
        value_wei,
        gas_limit,
        chain_id,
        gas_price_wei,
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei,
        input,
    })
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

fn parse_hex_u128(value: &str) -> Option<u128> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Some(0);
    }
    u128::from_str_radix(trimmed, 16).ok()
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut rendered = String::new();
    for (index, cause) in err.chain().enumerate() {
        if index > 0 {
            rendered.push_str(": ");
        }
        rendered.push_str(&cause.to_string());
    }
    rendered
}

fn format_fixed_hex(bytes: &[u8]) -> String {
    let mut out = String::from("0x");
    out.reserve(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn format_optional_fixed_hex(bytes: Option<&[u8]>) -> String {
    bytes
        .map(format_fixed_hex)
        .unwrap_or_else(|| "null".to_owned())
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn format_optional_u128(value: Option<u128>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn format_method_selector_hex(selector: Option<[u8; 4]>) -> String {
    match selector {
        Some(selector) => format!(
            "0x{:02x}{:02x}{:02x}{:02x}",
            selector[0], selector[1], selector[2], selector[3]
        ),
        None => "null".to_owned(),
    }
}

fn current_seq_hi(storage: &Arc<RwLock<InMemoryStorage>>) -> u64 {
    storage
        .read()
        .ok()
        .and_then(|store| store.latest_seq_id())
        .unwrap_or(0)
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use serde_json::json;

    #[test]
    fn default_live_rpc_config_has_primary_and_fallback_public_endpoints() {
        let config = LiveRpcConfig::default();
        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.endpoints[0].ws_url, PRIMARY_PUBLIC_WS_URL);
        assert_eq!(config.endpoints[0].http_url, PRIMARY_PUBLIC_HTTP_URL);
        assert_eq!(config.endpoints[1].ws_url, FALLBACK_PUBLIC_WS_URL);
        assert_eq!(config.endpoints[1].http_url, FALLBACK_PUBLIC_HTTP_URL);
        assert_eq!(config.source_id, SourceId::new("rpc-live"));
        assert!(config.max_seen_hashes >= 100);
    }

    #[test]
    fn format_error_chain_includes_context_and_root() {
        let err = anyhow!("root cause")
            .context("inner context")
            .context("outer context");

        let rendered = format_error_chain(&err);

        assert_eq!(rendered, "outer context: inner context: root cause");
    }

    #[test]
    fn rpc_tx_to_live_tx_parses_extended_fields() {
        let hash_hex = format!("0x{}", "11".repeat(32));
        let rpc_tx: RpcTransaction = serde_json::from_value(json!({
            "hash": hash_hex,
            "from": format!("0x{}", "22".repeat(20)),
            "to": format!("0x{}", "33".repeat(20)),
            "nonce": "0x2a",
            "type": "0x2",
            "input": "0xaabb",
            "value": "0x0de0b6b3a7640000",
            "gas": "0x5208",
            "chainId": "0x1",
            "gasPrice": "0x3b9aca00",
            "maxFeePerGas": "0x4a817c800",
            "maxPriorityFeePerGas": "0x77359400",
            "maxFeePerBlobGas": "0x3"
        }))
        .expect("decode rpc tx");

        let live = rpc_tx_to_live_tx(rpc_tx, &format!("0x{}", "11".repeat(32))).expect("map tx");

        assert_eq!(live.hash, [0x11; 32]);
        assert_eq!(live.sender, [0x22; 20]);
        assert_eq!(live.to, Some([0x33; 20]));
        assert_eq!(live.nonce, 42);
        assert_eq!(live.tx_type, 2);
        assert_eq!(live.input, vec![0xaa, 0xbb]);
        assert_eq!(live.value_wei, Some(1_000_000_000_000_000_000));
        assert_eq!(live.gas_limit, Some(21_000));
        assert_eq!(live.chain_id, Some(1));
        assert_eq!(live.gas_price_wei, Some(1_000_000_000));
        assert_eq!(live.max_fee_per_gas_wei, Some(20_000_000_000));
        assert_eq!(live.max_priority_fee_per_gas_wei, Some(2_000_000_000));
        assert_eq!(live.max_fee_per_blob_gas_wei, Some(3));
    }

    #[test]
    fn build_opportunity_records_maps_ranked_candidates_to_storage_rows() {
        let uniswap_v2_router = [
            0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac,
            0xb4, 0xc6, 0x59, 0xf2, 0x48, 0x8d,
        ];
        let calldata = vec![0x38, 0xed, 0x17, 0x39, 1, 2, 3, 4, 5, 6, 7, 8];
        let decoded = TxDecoded {
            hash: [0x44; 32],
            tx_type: 2,
            sender: [0x55; 20],
            nonce: 42,
            chain_id: Some(1),
            to: Some(uniswap_v2_router),
            value_wei: Some(1_000_000_000_000_000),
            gas_limit: Some(320_000),
            gas_price_wei: None,
            max_fee_per_gas_wei: Some(45_000_000_000),
            max_priority_fee_per_gas_wei: Some(7_000_000_000),
            max_fee_per_blob_gas_wei: None,
            calldata_len: Some(calldata.len() as u32),
        };

        let detected_unix_ms = 1_700_000_001_234;
        let records = build_opportunity_records(&decoded, &calldata, detected_unix_ms);

        assert!(!records.is_empty());
        assert!(
            records
                .windows(2)
                .all(|pair| pair[0].score >= pair[1].score)
        );
        assert!(
            records
                .iter()
                .all(|record| record.detected_unix_ms == detected_unix_ms)
        );
        assert_eq!(records[0].feature_engine_version, feature_engine::version());
        assert_eq!(records[0].scorer_version, searcher::scorer_version());
        assert!(records[0].strategy_version.starts_with("strategy."));
        assert!(!records[0].reasons.is_empty());
    }

    #[test]
    fn build_rule_versioned_events_emits_opp_sim_and_bundle_with_same_versions() {
        let tx_hash = [0x11; 32];
        let opportunity = OpportunityRecord {
            tx_hash,
            strategy: "SandwichCandidate".to_owned(),
            score: 12_345,
            protocol: "uniswap-v2".to_owned(),
            category: "swap".to_owned(),
            feature_engine_version: "feature-engine.v9".to_owned(),
            scorer_version: "scorer.v3".to_owned(),
            strategy_version: "strategy.sandwich.v7".to_owned(),
            reasons: vec!["example-reason".to_owned()],
            detected_unix_ms: 1_700_000_001_000,
        };

        let events = build_rule_versioned_events(
            41,
            1_700_000_001_111,
            &SourceId::new("rpc-live"),
            &opportunity,
        );
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].seq_id, 41);
        assert_eq!(events[1].seq_id, 42);
        assert_eq!(events[2].seq_id, 43);

        match &events[0].payload {
            EventPayload::OppDetected(opp) => {
                assert_eq!(opp.hash, tx_hash);
                assert_eq!(
                    opp.feature_engine_version,
                    opportunity.feature_engine_version
                );
                assert_eq!(opp.scorer_version, opportunity.scorer_version);
                assert_eq!(opp.strategy_version, opportunity.strategy_version);
            }
            other => panic!("expected OppDetected payload, got {other:?}"),
        }

        match &events[1].payload {
            EventPayload::SimCompleted(sim) => {
                assert_eq!(sim.hash, tx_hash);
                assert_eq!(sim.status, "not_run");
                assert_eq!(
                    sim.feature_engine_version,
                    opportunity.feature_engine_version
                );
                assert_eq!(sim.scorer_version, opportunity.scorer_version);
                assert_eq!(sim.strategy_version, opportunity.strategy_version);
            }
            other => panic!("expected SimCompleted payload, got {other:?}"),
        }

        match &events[2].payload {
            EventPayload::BundleSubmitted(bundle) => {
                assert_eq!(bundle.hash, tx_hash);
                assert!(!bundle.accepted);
                assert_eq!(bundle.relay, "not_submitted");
                assert_eq!(
                    bundle.feature_engine_version,
                    opportunity.feature_engine_version
                );
                assert_eq!(bundle.scorer_version, opportunity.scorer_version);
                assert_eq!(bundle.strategy_version, opportunity.strategy_version);
            }
            other => panic!("expected BundleSubmitted payload, got {other:?}"),
        }
    }
}
