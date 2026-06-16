use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use serde::Serialize;

use crate::{config, ApiError};

#[derive(Clone, Debug, Serialize)]
pub struct QuotaLimits {
    pub requests_per_minute: u32,
    pub requests_per_day: u32,
    pub tokens_per_day: u64,
    pub tokens_per_month: u64,
    pub max_tokens_per_request: u32,
}

impl QuotaLimits {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            requests_per_minute: config::env_u32("RELAY_REQUESTS_PER_MINUTE", 20)?,
            requests_per_day: config::env_u32("RELAY_REQUESTS_PER_DAY", 1_000)?,
            tokens_per_day: config::env_u64("RELAY_TOKENS_PER_DAY", 200_000)?,
            tokens_per_month: config::env_u64("RELAY_TOKENS_PER_MONTH", 3_000_000)?,
            max_tokens_per_request: config::env_u32("RELAY_MAX_TOKENS_PER_REQUEST", 4_096)?,
        })
    }
}

pub struct QuotaManager {
    limits: QuotaLimits,
    counters: Mutex<HashMap<String, ClientCounter>>,
}

impl QuotaManager {
    pub fn new(limits: QuotaLimits) -> Self {
        Self {
            limits,
            counters: Mutex::new(HashMap::new()),
        }
    }

    pub fn precheck(&self, client_id: &str, requested_max_tokens: u32) -> Result<(), ApiError> {
        if requested_max_tokens > self.limits.max_tokens_per_request {
            return Err(ApiError::too_many_requests(
                "max_tokens_exceeded",
                "proxy.max_tokens_exceeded",
            ));
        }

        let now = SystemTime::now();
        let mut counters = self.counters.lock().expect("quota mutex poisoned");
        let counter = counters.entry(client_id.to_owned()).or_default();
        counter.roll_windows(now);

        if counter.minute_requests >= self.limits.requests_per_minute {
            return Err(ApiError::too_many_requests(
                "requests_per_minute_exceeded",
                "proxy.requests_per_minute_exceeded",
            ));
        }
        if counter.day_requests >= self.limits.requests_per_day {
            return Err(ApiError::too_many_requests(
                "requests_per_day_exceeded",
                "proxy.requests_per_day_exceeded",
            ));
        }
        if counter.day_tokens >= self.limits.tokens_per_day {
            return Err(ApiError::too_many_requests(
                "tokens_per_day_exceeded",
                "proxy.tokens_per_day_exceeded",
            ));
        }
        if counter.month_tokens >= self.limits.tokens_per_month {
            return Err(ApiError::too_many_requests(
                "tokens_per_month_exceeded",
                "proxy.tokens_per_month_exceeded",
            ));
        }

        counter.minute_requests += 1;
        counter.day_requests += 1;
        Ok(())
    }

    pub fn settle(&self, client_id: &str, total_tokens: u64) {
        let now = SystemTime::now();
        let mut counters = self.counters.lock().expect("quota mutex poisoned");
        let counter = counters.entry(client_id.to_owned()).or_default();
        counter.roll_windows(now);
        counter.day_tokens = counter.day_tokens.saturating_add(total_tokens);
        counter.month_tokens = counter.month_tokens.saturating_add(total_tokens);
        counter.successful_requests = counter.successful_requests.saturating_add(1);
    }

    pub fn record_failed_request(&self, client_id: &str) {
        let mut counters = self.counters.lock().expect("quota mutex poisoned");
        let counter = counters.entry(client_id.to_owned()).or_default();
        counter.failed_requests = counter.failed_requests.saturating_add(1);
    }

    pub fn snapshot(&self, client_id: &str) -> QuotaSnapshot {
        let now = SystemTime::now();
        let mut counters = self.counters.lock().expect("quota mutex poisoned");
        let counter = counters.entry(client_id.to_owned()).or_default();
        counter.roll_windows(now);

        QuotaSnapshot {
            minute_requests: counter.minute_requests,
            day_requests: counter.day_requests,
            successful_requests: counter.successful_requests,
            failed_requests: counter.failed_requests,
            day_tokens: counter.day_tokens,
            month_tokens: counter.month_tokens,
            remaining_minute_requests: self
                .limits
                .requests_per_minute
                .saturating_sub(counter.minute_requests),
            remaining_day_requests: self
                .limits
                .requests_per_day
                .saturating_sub(counter.day_requests),
            remaining_day_tokens: self
                .limits
                .tokens_per_day
                .saturating_sub(counter.day_tokens),
            remaining_month_tokens: self
                .limits
                .tokens_per_month
                .saturating_sub(counter.month_tokens),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ClientCounter {
    minute_started_at: Option<SystemTime>,
    day_started_at: Option<SystemTime>,
    month_started_at: Option<SystemTime>,
    minute_requests: u32,
    day_requests: u32,
    successful_requests: u64,
    failed_requests: u64,
    day_tokens: u64,
    month_tokens: u64,
}

impl ClientCounter {
    fn roll_windows(&mut self, now: SystemTime) {
        if should_reset(self.minute_started_at, now, Duration::from_secs(60)) {
            self.minute_started_at = Some(now);
            self.minute_requests = 0;
        }
        if should_reset(self.day_started_at, now, Duration::from_secs(24 * 60 * 60)) {
            self.day_started_at = Some(now);
            self.day_requests = 0;
            self.day_tokens = 0;
        }
        if should_reset(
            self.month_started_at,
            now,
            Duration::from_secs(30 * 24 * 60 * 60),
        ) {
            self.month_started_at = Some(now);
            self.month_tokens = 0;
        }
    }
}

fn should_reset(started_at: Option<SystemTime>, now: SystemTime, window: Duration) -> bool {
    match started_at {
        Some(started_at) => now
            .duration_since(started_at)
            .map_or(true, |elapsed| elapsed >= window),
        None => true,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QuotaSnapshot {
    pub minute_requests: u32,
    pub day_requests: u32,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub day_tokens: u64,
    pub month_tokens: u64,
    pub remaining_minute_requests: u32,
    pub remaining_day_requests: u32,
    pub remaining_day_tokens: u64,
    pub remaining_month_tokens: u64,
}
