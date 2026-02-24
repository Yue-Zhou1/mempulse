#![forbid(unsafe_code)]

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockTemplate {
    pub slot: u64,
    pub parent_hash: String,
    pub block_hash: String,
    pub builder_pubkey: String,
    pub tx_count: u32,
    pub gas_used: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayAttemptTrace {
    pub attempt: u32,
    pub endpoint: String,
    pub http_status: Option<u16>,
    pub error: Option<String>,
    pub latency_ms: u64,
    pub backoff_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayDryRunResult {
    pub relay_url: String,
    pub accepted: bool,
    pub final_state: String,
    pub attempts: Vec<RelayAttemptTrace>,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelayClientConfig {
    pub relay_url: String,
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
    pub request_timeout_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct RelayDryRunStatus {
    pub latest: Option<RelayDryRunResult>,
    pub total_submissions: u64,
    pub total_accepted: u64,
    pub total_failed: u64,
}

impl RelayDryRunStatus {
    pub fn record(&mut self, result: RelayDryRunResult) {
        self.total_submissions = self.total_submissions.saturating_add(1);
        if result.accepted {
            self.total_accepted = self.total_accepted.saturating_add(1);
        } else {
            self.total_failed = self.total_failed.saturating_add(1);
        }
        self.latest = Some(result);
    }
}

#[derive(Clone)]
pub struct RelayClient {
    http: reqwest::Client,
    config: RelayClientConfig,
}

impl RelayClient {
    pub fn new(config: RelayClientConfig) -> Result<Self> {
        if config.relay_url.trim().is_empty() {
            bail!("relay_url must not be empty");
        }
        let timeout = Duration::from_millis(config.request_timeout_ms.max(1));
        let http = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Self { http, config })
    }

    pub async fn submit_dry_run(&self, template: &BlockTemplate) -> Result<RelayDryRunResult> {
        let started = unix_ms_now();
        let mut attempts = Vec::new();

        for attempt in 0..=self.config.max_retries {
            let attempt_idx = attempt.saturating_add(1);
            let begin = Instant::now();
            let response = self
                .http
                .post(&self.config.relay_url)
                .json(template)
                .send()
                .await;
            let elapsed = begin.elapsed().as_millis() as u64;

            match response {
                Ok(response) => {
                    let status = response.status();
                    let is_success = status.is_success();
                    let backoff_ms = if is_success || attempt == self.config.max_retries {
                        0
                    } else {
                        backoff_delay_ms(self.config.initial_backoff_ms, attempt)
                    };
                    attempts.push(RelayAttemptTrace {
                        attempt: attempt_idx,
                        endpoint: self.config.relay_url.clone(),
                        http_status: Some(status.as_u16()),
                        error: None,
                        latency_ms: elapsed,
                        backoff_ms,
                    });

                    if is_success {
                        return Ok(RelayDryRunResult {
                            relay_url: self.config.relay_url.clone(),
                            accepted: true,
                            final_state: "accepted".to_owned(),
                            attempts,
                            started_unix_ms: started,
                            finished_unix_ms: unix_ms_now(),
                        });
                    }
                    if backoff_ms > 0 {
                        sleep(Duration::from_millis(backoff_ms)).await;
                    }
                }
                Err(error) => {
                    let backoff_ms = if attempt == self.config.max_retries {
                        0
                    } else {
                        backoff_delay_ms(self.config.initial_backoff_ms, attempt)
                    };
                    attempts.push(RelayAttemptTrace {
                        attempt: attempt_idx,
                        endpoint: self.config.relay_url.clone(),
                        http_status: None,
                        error: Some(error.to_string()),
                        latency_ms: elapsed,
                        backoff_ms,
                    });
                    if backoff_ms > 0 {
                        sleep(Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
        }

        Ok(RelayDryRunResult {
            relay_url: self.config.relay_url.clone(),
            accepted: false,
            final_state: "exhausted".to_owned(),
            attempts,
            started_unix_ms: started,
            finished_unix_ms: unix_ms_now(),
        })
    }
}

fn backoff_delay_ms(initial_backoff_ms: u64, retry_index: u32) -> u64 {
    let retry_shift = retry_index.min(16);
    initial_backoff_ms
        .max(1)
        .saturating_mul(1_u64 << retry_shift)
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
