//! Shared machine contracts for the Feishu and Lark Open Platform adapters.
//!
//! The two products expose equivalent message shapes, but runtime identity,
//! credentials, API roots, locale, rate buckets, and delivery receipts must
//! remain isolated.

use crate::channel_capabilities::ChannelAdapterKind;
use crate::channel_provider_error::{ChannelProviderError, ChannelProviderFailureClass};
use crate::types::ChannelKind;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const OPEN_PLATFORM_TEXT_CONTENT_MAX_BYTES: usize = 150 * 1024;
pub const OPEN_PLATFORM_STRUCTURED_CONTENT_MAX_BYTES: usize = 30 * 1024;
pub const OPEN_PLATFORM_TARGET_QPS: u32 = 5;
pub const OPEN_PLATFORM_TARGET_MIN_INTERVAL_MILLIS: u64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPlatformRegion {
    Feishu,
    Lark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPlatformContract {
    pub region: OpenPlatformRegion,
    pub channel: ChannelKind,
    pub adapter: ChannelAdapterKind,
    pub source_adapter: &'static str,
    pub rate_bucket_namespace: &'static str,
    pub receipt_namespace: &'static str,
    pub message_source_ref: &'static str,
    pub file_source_ref: &'static str,
}

const FEISHU_CONTRACT: OpenPlatformContract = OpenPlatformContract {
    region: OpenPlatformRegion::Feishu,
    channel: ChannelKind::Feishu,
    adapter: ChannelAdapterKind::FeishuOpenPlatform,
    source_adapter: "feishu_open_platform",
    rate_bucket_namespace: "feishu_open_platform_target",
    receipt_namespace: "feishu_open_platform_delivery",
    message_source_ref:
        "https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create",
    file_source_ref:
        "https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/file/create",
};

const LARK_CONTRACT: OpenPlatformContract = OpenPlatformContract {
    region: OpenPlatformRegion::Lark,
    channel: ChannelKind::Lark,
    adapter: ChannelAdapterKind::LarkOpenPlatform,
    source_adapter: "lark_open_platform",
    rate_bucket_namespace: "lark_open_platform_target",
    receipt_namespace: "lark_open_platform_delivery",
    message_source_ref: "https://open.larksuite.com/document/server-docs/im-v1/message/create",
    file_source_ref: "https://open.larksuite.com/document/server-docs/im-v1/file/create",
};

pub const fn open_platform_contract(region: OpenPlatformRegion) -> &'static OpenPlatformContract {
    match region {
        OpenPlatformRegion::Feishu => &FEISHU_CONTRACT,
        OpenPlatformRegion::Lark => &LARK_CONTRACT,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPlatformMessageType {
    Text,
    Post,
    Interactive,
    Image,
    Media,
    Audio,
    File,
}

impl OpenPlatformMessageType {
    pub fn from_provider_token(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "post" => Some(Self::Post),
            "interactive" => Some(Self::Interactive),
            "image" => Some(Self::Image),
            "media" => Some(Self::Media),
            "audio" => Some(Self::Audio),
            "file" => Some(Self::File),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Post => "post",
            Self::Interactive => "interactive",
            Self::Image => "image",
            Self::Media => "media",
            Self::Audio => "audio",
            Self::File => "file",
        }
    }

    pub const fn content_max_bytes(self) -> usize {
        match self {
            Self::Post | Self::Interactive => OPEN_PLATFORM_STRUCTURED_CONTENT_MAX_BYTES,
            Self::Text | Self::Image | Self::Media | Self::Audio | Self::File => {
                OPEN_PLATFORM_TEXT_CONTENT_MAX_BYTES
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPlatformContentError {
    pub message_type: OpenPlatformMessageType,
    pub actual_bytes: usize,
    pub max_bytes: usize,
}

impl OpenPlatformContentError {
    pub const fn error_code(self) -> &'static str {
        "channel_open_platform_content_too_large"
    }

    pub const fn message_key(self) -> &'static str {
        "channel.error.payload_too_large"
    }
}

pub fn validate_open_platform_content(
    message_type: OpenPlatformMessageType,
    content: &str,
) -> Result<usize, OpenPlatformContentError> {
    let actual_bytes = content.len();
    let max_bytes = message_type.content_max_bytes();
    if actual_bytes > max_bytes {
        return Err(OpenPlatformContentError {
            message_type,
            actual_bytes,
            max_bytes,
        });
    }
    Ok(actual_bytes)
}

fn json_string_char_bytes(value: char) -> usize {
    match value {
        '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
        '\u{0000}'..='\u{001f}' => 6,
        _ => value.len_utf8(),
    }
}

/// Split plain text so every serialized `{\"text\": ...}` content value is
/// within the provider byte limit. `soft_char_limit` keeps product-configured
/// readability chunking without treating character count as provider authority.
pub fn chunk_open_platform_text(
    text: &str,
    soft_char_limit: usize,
) -> Result<Vec<String>, OpenPlatformContentError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }

    const EMPTY_TEXT_CONTENT_BYTES: usize = 11;
    let max_bytes = OpenPlatformMessageType::Text.content_max_bytes();
    let soft_char_limit = soft_char_limit.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_bytes = EMPTY_TEXT_CONTENT_BYTES;
    let mut current_chars = 0usize;

    for value in text.chars() {
        let value_bytes = json_string_char_bytes(value);
        if !current.is_empty()
            && (current_chars >= soft_char_limit || current_bytes + value_bytes > max_bytes)
        {
            chunks.push(std::mem::take(&mut current));
            current_bytes = EMPTY_TEXT_CONTENT_BYTES;
            current_chars = 0;
        }
        if current_bytes + value_bytes > max_bytes {
            return Err(OpenPlatformContentError {
                message_type: OpenPlatformMessageType::Text,
                actual_bytes: current_bytes + value_bytes,
                max_bytes,
            });
        }
        current.push(value);
        current_bytes += value_bytes;
        current_chars += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    for chunk in &chunks {
        let content = serde_json::json!({ "text": chunk }).to_string();
        validate_open_platform_content(OpenPlatformMessageType::Text, &content)?;
    }
    Ok(chunks)
}

pub fn scoped_open_platform_rate_bucket(region: OpenPlatformRegion, receive_id: &str) -> String {
    format!(
        "{}:{}",
        open_platform_contract(region).rate_bucket_namespace,
        receive_id.trim()
    )
}

pub fn scoped_open_platform_receipt_key(
    region: OpenPlatformRegion,
    idempotency_key: &str,
) -> String {
    format!(
        "{}:{}",
        open_platform_contract(region).receipt_namespace,
        idempotency_key.trim()
    )
}

#[derive(Debug, Default)]
pub struct OpenPlatformTargetRateLimiter {
    next_allowed_by_bucket: tokio::sync::Mutex<HashMap<String, Instant>>,
}

impl OpenPlatformTargetRateLimiter {
    fn reserve_at(
        buckets: &mut HashMap<String, Instant>,
        bucket: String,
        now: Instant,
    ) -> Duration {
        if buckets.len() > 4096 {
            buckets.retain(|_, next_allowed| *next_allowed >= now);
        }
        let scheduled = buckets
            .get(&bucket)
            .copied()
            .filter(|next_allowed| *next_allowed > now)
            .unwrap_or(now);
        buckets.insert(
            bucket,
            scheduled + Duration::from_millis(OPEN_PLATFORM_TARGET_MIN_INTERVAL_MILLIS),
        );
        scheduled.saturating_duration_since(now)
    }

    pub async fn acquire(&self, region: OpenPlatformRegion, receive_id: &str) {
        let bucket = scoped_open_platform_rate_bucket(region, receive_id);
        let delay = {
            let mut buckets = self.next_allowed_by_bucket.lock().await;
            Self::reserve_at(&mut buckets, bucket, Instant::now())
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
}

static PROCESS_OPEN_PLATFORM_RATE_LIMITER: OnceLock<OpenPlatformTargetRateLimiter> =
    OnceLock::new();

pub fn process_open_platform_rate_limiter() -> &'static OpenPlatformTargetRateLimiter {
    PROCESS_OPEN_PLATFORM_RATE_LIMITER.get_or_init(OpenPlatformTargetRateLimiter::default)
}

struct CachedOpenPlatformToken {
    value: String,
    expires_at_secs: u64,
}

#[derive(Default)]
pub struct OpenPlatformTokenCache {
    cached: tokio::sync::Mutex<Option<CachedOpenPlatformToken>>,
}

static PROCESS_OPEN_PLATFORM_TOKEN_CACHES: OnceLock<
    std::sync::Mutex<HashMap<String, std::sync::Arc<OpenPlatformTokenCache>>>,
> = OnceLock::new();

pub fn process_open_platform_token_cache(
    region: OpenPlatformRegion,
    api_base_url: &str,
    app_id: &str,
) -> std::sync::Arc<OpenPlatformTokenCache> {
    let scope = format!(
        "{}:{}:{}",
        open_platform_contract(region).source_adapter,
        api_base_url.trim_end_matches('/'),
        app_id.trim()
    );
    let caches =
        PROCESS_OPEN_PLATFORM_TOKEN_CACHES.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut caches = caches
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    caches
        .entry(scope)
        .or_insert_with(|| std::sync::Arc::new(OpenPlatformTokenCache::default()))
        .clone()
}

impl OpenPlatformTokenCache {
    pub async fn token_or_refresh<F, Fut, E>(&self, now_secs: u64, refresh: F) -> Result<String, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(String, u64), E>>,
    {
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref() {
            if token.expires_at_secs > now_secs.saturating_add(60) {
                return Ok(token.value.clone());
            }
        }

        let (value, valid_for_secs) = refresh().await?;
        let expires_at_secs = now_secs.saturating_add(valid_for_secs);
        *cached = Some(CachedOpenPlatformToken {
            value: value.clone(),
            expires_at_secs,
        });
        Ok(value)
    }
}

pub fn classify_open_platform_provider_code(
    provider_error_code: &str,
) -> Option<ChannelProviderFailureClass> {
    match provider_error_code {
        "234002" | "99991663" | "99991664" => Some(ChannelProviderFailureClass::Authentication),
        "230002" | "230006" | "230013" | "230017" | "230018" | "234007" => {
            Some(ChannelProviderFailureClass::PermissionDenied)
        }
        "230019" => Some(ChannelProviderFailureClass::TargetNotFound),
        "230020" | "11232" | "11233" | "99991400" => Some(ChannelProviderFailureClass::RateLimited),
        "99991403" => Some(ChannelProviderFailureClass::QuotaExhausted),
        "230001" | "230025" | "234001" | "234006" | "234010" => {
            Some(ChannelProviderFailureClass::PayloadRejected)
        }
        "232096" | "234041" | "234042" => Some(ChannelProviderFailureClass::ProviderUnavailable),
        _ => None,
    }
}

fn open_platform_provider_code(response_body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(response_body).ok()?;
    let code = value.get("code")?;
    let normalized = if let Some(number) = code.as_i64() {
        number.to_string()
    } else {
        code.as_str()?.trim().to_string()
    };
    (!normalized.is_empty()
        && normalized.len() <= 32
        && normalized.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some(normalized)
}

pub fn open_platform_provider_error(
    region: OpenPlatformRegion,
    operation: &str,
    status_code: u16,
    response_body: &str,
) -> ChannelProviderError {
    let contract = open_platform_contract(region);
    if let Some(provider_error_code) = open_platform_provider_code(response_body) {
        if let Some(failure_class) = classify_open_platform_provider_code(&provider_error_code) {
            return ChannelProviderError::from_machine_failure(
                contract.source_adapter,
                operation,
                failure_class,
                Some(status_code),
                Some(&provider_error_code),
                None,
                response_body,
            );
        }
    }
    ChannelProviderError::from_http_response(
        contract.source_adapter,
        operation,
        status_code,
        response_body,
    )
}

pub fn decode_open_platform_response(
    region: OpenPlatformRegion,
    operation: &str,
    status_code: u16,
    response_body: &str,
) -> Result<serde_json::Value, ChannelProviderError> {
    if !(200..300).contains(&status_code) {
        return Err(open_platform_provider_error(
            region,
            operation,
            status_code,
            response_body,
        ));
    }
    let value: serde_json::Value = serde_json::from_str(response_body).map_err(|_| {
        ChannelProviderError::invalid_response(
            open_platform_contract(region).source_adapter,
            operation,
            "response_json_invalid",
        )
    })?;
    let code = value
        .get("code")
        .and_then(|code| {
            code.as_i64()
                .or_else(|| code.as_str().and_then(|value| value.parse::<i64>().ok()))
        })
        .ok_or_else(|| {
            ChannelProviderError::invalid_response(
                open_platform_contract(region).source_adapter,
                operation,
                "response_code_missing",
            )
        })?;
    if code != 0 {
        return Err(open_platform_provider_error(
            region,
            operation,
            status_code,
            response_body,
        ));
    }
    Ok(value)
}

pub fn open_platform_message_id(
    region: OpenPlatformRegion,
    operation: &str,
    status_code: u16,
    response_body: &str,
) -> Result<String, ChannelProviderError> {
    let value = decode_open_platform_response(region, operation, status_code, response_body)?;
    value
        .pointer("/data/message_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message_id| {
            !message_id.is_empty()
                && message_id.len() <= 256
                && message_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
        .map(str::to_string)
        .ok_or_else(|| {
            ChannelProviderError::invalid_response(
                open_platform_contract(region).source_adapter,
                operation,
                "response_message_id_missing",
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPlatformOutboundMediaKind {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPlatformUploadEndpoint {
    Image,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPlatformMediaPlan {
    pub upload_endpoint: OpenPlatformUploadEndpoint,
    pub form_file_type: &'static str,
    pub message_type: OpenPlatformMessageType,
    pub key_name: &'static str,
    pub max_bytes: u64,
}

pub fn plan_open_platform_media(
    region: OpenPlatformRegion,
    kind: OpenPlatformOutboundMediaKind,
    path: &Path,
    actual_bytes: u64,
) -> OpenPlatformMediaPlan {
    use crate::channel_capabilities::ChannelCapabilityKind;
    use crate::channel_media_limits::required_channel_media_max_bytes;

    let contract = open_platform_contract(region);
    let image_max =
        required_channel_media_max_bytes(contract.adapter, ChannelCapabilityKind::SendImage);
    let file_max =
        required_channel_media_max_bytes(contract.adapter, ChannelCapabilityKind::SendFile);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    match kind {
        OpenPlatformOutboundMediaKind::Image if actual_bytes <= image_max => {
            OpenPlatformMediaPlan {
                upload_endpoint: OpenPlatformUploadEndpoint::Image,
                form_file_type: "message",
                message_type: OpenPlatformMessageType::Image,
                key_name: "image_key",
                max_bytes: image_max,
            }
        }
        OpenPlatformOutboundMediaKind::Image => OpenPlatformMediaPlan {
            upload_endpoint: OpenPlatformUploadEndpoint::File,
            form_file_type: "stream",
            message_type: OpenPlatformMessageType::File,
            key_name: "file_key",
            max_bytes: file_max,
        },
        OpenPlatformOutboundMediaKind::Video if extension.eq_ignore_ascii_case("mp4") => {
            OpenPlatformMediaPlan {
                upload_endpoint: OpenPlatformUploadEndpoint::File,
                form_file_type: "mp4",
                message_type: OpenPlatformMessageType::Media,
                key_name: "file_key",
                max_bytes: file_max,
            }
        }
        OpenPlatformOutboundMediaKind::Audio if extension.eq_ignore_ascii_case("opus") => {
            OpenPlatformMediaPlan {
                upload_endpoint: OpenPlatformUploadEndpoint::File,
                form_file_type: "opus",
                message_type: OpenPlatformMessageType::Audio,
                key_name: "file_key",
                max_bytes: file_max,
            }
        }
        OpenPlatformOutboundMediaKind::Video
        | OpenPlatformOutboundMediaKind::Audio
        | OpenPlatformOutboundMediaKind::File => OpenPlatformMediaPlan {
            upload_endpoint: OpenPlatformUploadEndpoint::File,
            form_file_type: "stream",
            message_type: OpenPlatformMessageType::File,
            key_name: "file_key",
            max_bytes: file_max,
        },
    }
}

pub fn preflight_open_platform_media(
    region: OpenPlatformRegion,
    operation: &str,
    path: &Path,
    max_bytes: u64,
) -> Result<u64, ChannelProviderError> {
    crate::channel_media_limits::preflight_local_media_file(path, max_bytes).map_err(|error| {
        ChannelProviderError::from_machine_failure(
            open_platform_contract(region).source_adapter,
            operation,
            ChannelProviderFailureClass::PayloadRejected,
            None,
            Some(error.error_code()),
            None,
            &format!(
                "{}:{}",
                error.actual_bytes.unwrap_or_default(),
                error.max_bytes
            ),
        )
    })
}

#[cfg(test)]
#[path = "channel_open_platform_tests.rs"]
mod tests;
