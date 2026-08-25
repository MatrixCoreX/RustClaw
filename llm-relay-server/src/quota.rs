use std::{collections::HashMap, sync::Mutex, time::Instant};

use serde::Serialize;

use crate::{config, ApiError};

#[derive(Clone, Debug, Serialize)]
pub struct QuotaLimits {
    pub requests_per_minute: u32,
    pub requests_per_day: u32,
    pub tokens_per_day: u64,
    pub max_tokens_per_request: u32,
}

impl QuotaLimits {
    pub fn from_env() -> anyhow::Result<Self> {
        let limits = Self {
            requests_per_minute: config::env_u32("RELAY_REQUESTS_PER_MINUTE", 20)?,
            requests_per_day: config::env_u32("RELAY_REQUESTS_PER_DAY", 100)?,
            tokens_per_day: config::env_u64("RELAY_TOKENS_PER_DAY", 100_000_000)?,
            max_tokens_per_request: config::env_u32("RELAY_MAX_TOKENS_PER_REQUEST", 16_384)?,
        };
        anyhow::ensure!(
            limits.requests_per_minute > 0,
            "RELAY_REQUESTS_PER_MINUTE must be positive"
        );
        anyhow::ensure!(
            limits.requests_per_day > 0,
            "RELAY_REQUESTS_PER_DAY must be positive"
        );
        anyhow::ensure!(
            limits.tokens_per_day > 0,
            "RELAY_TOKENS_PER_DAY must be positive"
        );
        anyhow::ensure!(
            limits.max_tokens_per_request > 0,
            "RELAY_MAX_TOKENS_PER_REQUEST must be positive"
        );
        Ok(limits)
    }
}

pub struct MinuteRateLimiter {
    request_limit: u32,
    counters: Mutex<HashMap<String, MinuteCounter>>,
}

impl MinuteRateLimiter {
    pub fn new(request_limit: u32) -> Self {
        Self {
            request_limit,
            counters: Mutex::new(HashMap::new()),
        }
    }

    pub fn reserve(&self, key_id: &str) -> Result<(), ApiError> {
        let now = Instant::now();
        let mut counters = self.counters.lock().expect("rate-limit mutex poisoned");
        let counter = counters.entry(key_id.to_owned()).or_insert(MinuteCounter {
            started_at: now,
            requests: 0,
        });
        if now.duration_since(counter.started_at).as_secs() >= 60 {
            counter.started_at = now;
            counter.requests = 0;
        }
        if counter.requests >= self.request_limit {
            return Err(ApiError::too_many_requests(
                "requests_per_minute_exceeded",
                "proxy.requests_per_minute_exceeded",
            ));
        }
        counter.requests += 1;
        Ok(())
    }
}

struct MinuteCounter {
    started_at: Instant,
    requests: u32,
}
