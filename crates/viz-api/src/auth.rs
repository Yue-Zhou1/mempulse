//! API key and rate-limiting helpers for the HTTP server.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[derive(Clone, Debug)]
/// API authentication and rate-limit settings loaded from the environment.
pub struct ApiAuthConfig {
    pub enabled: bool,
    pub api_keys: HashSet<String>,
    pub requests_per_minute: u32,
}

impl Default for ApiAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_keys: HashSet::new(),
            requests_per_minute: 600,
        }
    }
}

impl ApiAuthConfig {
    /// Builds auth configuration from environment variables.
    pub fn from_env() -> Self {
        let enabled = std::env::var("VIZ_API_AUTH_ENABLED")
            .ok()
            .map(|value| parse_env_bool(value.trim()))
            .unwrap_or(false);
        let api_keys = std::env::var("VIZ_API_API_KEYS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let requests_per_minute = std::env::var("VIZ_API_RATE_LIMIT_PER_MINUTE")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(600);

        Self {
            enabled,
            api_keys,
            requests_per_minute,
        }
    }

    /// Returns whether an API key is valid for the current config.
    pub fn validates_key(&self, key: &str) -> bool {
        !self.enabled || self.api_keys.contains(key)
    }
}

#[derive(Clone, Debug)]
/// Per-key token-bucket rate limiter.
pub struct ApiRateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Arc<RwLock<HashMap<String, TokenBucket>>>,
}

impl ApiRateLimiter {
    /// Creates a rate limiter with the provided requests-per-minute allowance.
    pub fn new(requests_per_minute: u32) -> Self {
        let capacity = requests_per_minute.max(1) as f64;
        Self {
            capacity,
            refill_per_sec: capacity / 60.0,
            buckets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Returns whether a request for the given key should be allowed.
    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut guard = match self.buckets.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let bucket = guard.entry(key.to_owned()).or_insert(TokenBucket {
            tokens: self.capacity,
            last_refill: now,
        });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        let replenished = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.tokens = replenished;
        bucket.last_refill = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }
}

#[derive(Clone, Debug)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

fn parse_env_bool(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
