use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{header, HeaderMap, Method};
use claw_core::config::WebdRequestLimitsConfig;

const MAX_RATE_KEYS: usize = 20_000;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum RequestClass {
    General,
    Login,
    TaskSubmit,
    Upload,
    HighCost,
    Sse,
}

#[derive(Clone, Debug)]
pub(super) struct LimitRejection {
    pub(super) error_code: &'static str,
    pub(super) retry_after_seconds: u64,
}

#[derive(Clone)]
pub(super) struct RequestLimits {
    inner: Arc<RequestLimitsInner>,
}

struct RequestLimitsInner {
    config: WebdRequestLimitsConfig,
    max_upload_body_bytes: usize,
    active: Mutex<ActiveCounts>,
    rates: Mutex<RateWindows>,
}

#[derive(Default)]
struct ActiveCounts {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
    per_session: HashMap<String, usize>,
    sse_per_ip: HashMap<IpAddr, usize>,
    uploads: usize,
    high_cost: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum RateKey {
    Ip(IpAddr),
    Session(String),
    ClassIp(RequestClass, IpAddr),
}

#[derive(Clone, Copy, Debug)]
struct RateWindow {
    minute: u64,
    count: u32,
}

#[derive(Default)]
struct RateWindows {
    entries: HashMap<RateKey, RateWindow>,
}

pub(super) struct RequestLease {
    limits: RequestLimits,
    client_ip: IpAddr,
    session_id: Option<String>,
    class: RequestClass,
}

impl RequestLimits {
    pub(super) fn new(config: WebdRequestLimitsConfig, max_upload_body_bytes: usize) -> Self {
        Self {
            inner: Arc::new(RequestLimitsInner {
                config,
                max_upload_body_bytes: max_upload_body_bytes.max(1),
                active: Mutex::new(ActiveCounts::default()),
                rates: Mutex::new(RateWindows::default()),
            }),
        }
    }

    pub(super) fn try_acquire(
        &self,
        client_ip: IpAddr,
        session_id: Option<&str>,
        class: RequestClass,
        now_unix: u64,
    ) -> Result<RequestLease, LimitRejection> {
        let session_id = session_id.map(ToOwned::to_owned);
        {
            let mut active = self.inner.active.lock().map_err(|_| LimitRejection {
                error_code: "webd_request_limiter_unavailable",
                retry_after_seconds: 1,
            })?;
            let config = &self.inner.config;
            if active.total >= config.global_concurrency.max(1) {
                return Err(concurrency_rejection("webd_global_concurrency_limited"));
            }
            if active.per_ip.get(&client_ip).copied().unwrap_or(0)
                >= config.per_ip_concurrency.max(1)
            {
                return Err(concurrency_rejection("webd_ip_concurrency_limited"));
            }
            if let Some(session_id) = session_id.as_ref() {
                if active.per_session.get(session_id).copied().unwrap_or(0)
                    >= config.per_session_concurrency.max(1)
                {
                    return Err(concurrency_rejection("webd_session_concurrency_limited"));
                }
            }
            if class == RequestClass::Sse
                && active.sse_per_ip.get(&client_ip).copied().unwrap_or(0)
                    >= config.sse_per_ip_concurrency.max(1)
            {
                return Err(concurrency_rejection("webd_sse_concurrency_limited"));
            }
            if class == RequestClass::Upload && active.uploads >= config.upload_concurrency.max(1) {
                return Err(concurrency_rejection("webd_upload_concurrency_limited"));
            }
            if class == RequestClass::HighCost
                && active.high_cost >= config.high_cost_concurrency.max(1)
            {
                return Err(concurrency_rejection("webd_high_cost_concurrency_limited"));
            }

            active.total += 1;
            *active.per_ip.entry(client_ip).or_default() += 1;
            if let Some(session_id) = session_id.as_ref() {
                *active.per_session.entry(session_id.clone()).or_default() += 1;
            }
            if class == RequestClass::Sse {
                *active.sse_per_ip.entry(client_ip).or_default() += 1;
            }
            if class == RequestClass::Upload {
                active.uploads += 1;
            }
            if class == RequestClass::HighCost {
                active.high_cost += 1;
            }
        }

        let lease = RequestLease {
            limits: self.clone(),
            client_ip,
            session_id,
            class,
        };
        if let Err(rejection) =
            self.consume_rate(client_ip, lease.session_id.as_deref(), class, now_unix)
        {
            drop(lease);
            return Err(rejection);
        }
        Ok(lease)
    }

    pub(super) fn validate_headers(
        &self,
        headers: &HeaderMap,
        class: RequestClass,
    ) -> Result<(), &'static str> {
        let header_bytes = headers.iter().fold(0usize, |total, (name, value)| {
            total
                .saturating_add(name.as_str().len())
                .saturating_add(value.as_bytes().len())
        });
        if header_bytes > self.inner.config.max_header_bytes.max(1) {
            return Err("webd_request_headers_too_large");
        }

        for value in headers.get_all(header::CONTENT_ENCODING) {
            let value = value
                .to_str()
                .map_err(|_| "webd_request_content_encoding_unsupported")?;
            if value
                .split(',')
                .map(str::trim)
                .any(|encoding| !encoding.eq_ignore_ascii_case("identity"))
            {
                return Err("webd_request_content_encoding_unsupported");
            }
        }

        let mut content_lengths = headers.get_all(header::CONTENT_LENGTH).iter();
        let content_length = content_lengths.next().map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or("webd_content_length_invalid")
        });
        if content_lengths.next().is_some() {
            return Err("webd_content_length_invalid");
        }
        if let Some(content_length) = content_length.transpose()? {
            if content_length > self.body_limit(class) {
                return Err("webd_request_body_too_large");
            }
        }
        Ok(())
    }

    pub(super) fn body_limit(&self, class: RequestClass) -> usize {
        if matches!(class, RequestClass::TaskSubmit | RequestClass::Upload) {
            self.inner.max_upload_body_bytes
        } else {
            self.inner.config.max_json_body_bytes.max(1)
        }
    }

    pub(super) fn body_read_timeout(&self, class: RequestClass) -> Duration {
        let seconds = if matches!(class, RequestClass::TaskSubmit | RequestClass::Upload) {
            self.inner.config.upload_body_read_timeout_seconds
        } else {
            self.inner.config.body_read_timeout_seconds
        };
        Duration::from_secs(seconds.max(1))
    }

    pub(super) fn sse_max_lifetime(&self) -> Duration {
        Duration::from_secs(self.inner.config.sse_max_lifetime_seconds.max(1))
    }

    fn consume_rate(
        &self,
        client_ip: IpAddr,
        session_id: Option<&str>,
        class: RequestClass,
        now_unix: u64,
    ) -> Result<(), LimitRejection> {
        let config = &self.inner.config;
        let mut requirements = vec![(RateKey::Ip(client_ip), config.per_ip_rpm.max(1))];
        if let Some(session_id) = session_id {
            requirements.push((
                RateKey::Session(session_id.to_string()),
                config.per_session_rpm.max(1),
            ));
        }
        let class_limit = match class {
            RequestClass::Login => Some(config.login_per_ip_rpm.max(1)),
            RequestClass::TaskSubmit => Some(config.task_per_ip_rpm.max(1)),
            RequestClass::Upload => Some(config.upload_per_ip_rpm.max(1)),
            RequestClass::HighCost => Some(config.high_cost_per_ip_rpm.max(1)),
            RequestClass::General | RequestClass::Sse => None,
        };
        if let Some(limit) = class_limit {
            requirements.push((RateKey::ClassIp(class, client_ip), limit));
        }

        let minute = now_unix / 60;
        let retry_after_seconds = 60 - (now_unix % 60);
        let mut rates = self.inner.rates.lock().map_err(|_| LimitRejection {
            error_code: "webd_request_limiter_unavailable",
            retry_after_seconds: 1,
        })?;
        let missing = requirements
            .iter()
            .filter(|(key, _)| !rates.entries.contains_key(key))
            .count();
        if rates.entries.len().saturating_add(missing) > MAX_RATE_KEYS {
            rates
                .entries
                .retain(|_, window| window.minute.saturating_add(1) >= minute);
            if rates.entries.len().saturating_add(missing) > MAX_RATE_KEYS {
                return Err(LimitRejection {
                    error_code: "webd_rate_limiter_capacity_exceeded",
                    retry_after_seconds,
                });
            }
        }

        for (key, limit) in &requirements {
            if rates
                .entries
                .get(key)
                .filter(|window| window.minute == minute)
                .is_some_and(|window| window.count >= *limit)
            {
                return Err(LimitRejection {
                    error_code: rate_error_code(class),
                    retry_after_seconds,
                });
            }
        }
        for (key, _) in requirements {
            let window = rates
                .entries
                .entry(key)
                .or_insert(RateWindow { minute, count: 0 });
            if window.minute != minute {
                window.minute = minute;
                window.count = 0;
            }
            window.count = window.count.saturating_add(1);
        }
        Ok(())
    }
}

impl Drop for RequestLease {
    fn drop(&mut self) {
        let mut active = self
            .limits
            .inner
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active.total = active.total.saturating_sub(1);
        decrement_entry(&mut active.per_ip, &self.client_ip);
        if let Some(session_id) = self.session_id.as_ref() {
            decrement_entry(&mut active.per_session, session_id);
        }
        if self.class == RequestClass::Sse {
            decrement_entry(&mut active.sse_per_ip, &self.client_ip);
        }
        if self.class == RequestClass::Upload {
            active.uploads = active.uploads.saturating_sub(1);
        }
        if self.class == RequestClass::HighCost {
            active.high_cost = active.high_cost.saturating_sub(1);
        }
    }
}

fn decrement_entry<K>(entries: &mut HashMap<K, usize>, key: &K)
where
    K: Eq + std::hash::Hash,
{
    if let Some(count) = entries.get_mut(key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            entries.remove(key);
        }
    }
}

fn concurrency_rejection(error_code: &'static str) -> LimitRejection {
    LimitRejection {
        error_code,
        retry_after_seconds: 1,
    }
}

fn rate_error_code(class: RequestClass) -> &'static str {
    match class {
        RequestClass::Login => "webd_login_rate_limited",
        RequestClass::TaskSubmit => "webd_task_rate_limited",
        RequestClass::Upload => "webd_upload_rate_limited",
        RequestClass::HighCost => "webd_high_cost_rate_limited",
        RequestClass::General | RequestClass::Sse => "webd_request_rate_limited",
    }
}

pub(super) fn classify_request(method: &Method, path: &str) -> RequestClass {
    if is_task_event_stream(method, path) {
        return RequestClass::Sse;
    }
    if *method == Method::POST && path == "/v1/tasks" {
        return RequestClass::TaskSubmit;
    }
    if method_is_mutating(method) && is_upload_path(path) {
        return RequestClass::Upload;
    }
    if method_is_mutating(method) && is_high_cost_path(path) {
        return RequestClass::HighCost;
    }
    RequestClass::General
}

fn method_is_mutating(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn is_task_event_stream(method: &Method, path: &str) -> bool {
    if *method != Method::GET {
        return false;
    }
    path.strip_prefix("/v1/tasks/")
        .and_then(|suffix| suffix.strip_suffix("/events"))
        .is_some_and(|task_id| !task_id.is_empty() && !task_id.contains('/'))
}

fn is_upload_path(path: &str) -> bool {
    matches!(
        path,
        "/v1/skills/import/upload" | "/v1/memory/import/preview"
    )
}

fn is_high_cost_path(path: &str) -> bool {
    path == "/v1/llm/test"
        || path == "/v1/memory/vector/reindex"
        || path == "/v1/nni/bancor/trade"
        || path == "/v1/nni/assets/transfer"
        || path == "/v1/nni/device/action"
        || path.starts_with("/v1/nni/join/")
        || path.starts_with("/v1/nni/owner/")
        || path.starts_with("/v1/skills/store/")
        || path.starts_with("/v1/admin/workspace-update/")
        || path.starts_with("/v1/admin/system-dependencies/")
        || path.starts_with("/v1/admin/mcp/servers/") && path.ends_with("/test")
}

#[cfg(test)]
#[path = "request_limits_tests.rs"]
mod tests;
