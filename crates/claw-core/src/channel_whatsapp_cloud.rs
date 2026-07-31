use serde::{Deserialize, Serialize};

use crate::channel_provider_error::{ChannelProviderError, ChannelProviderFailureClass};

pub const WHATSAPP_CLOUD_SOURCE_ADAPTER: &str = "whatsapp_cloud";
pub const WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS: u64 = 24 * 60 * 60;
pub const WHATSAPP_ACCEPTED_DELIVERY_EVENT_SCHEMA_VERSION: u16 = 1;
pub const WHATSAPP_CLOUD_MEDIA_DOC_SOURCE: &str =
    "https://developers.facebook.com/docs/whatsapp/cloud-api/reference/media";
pub const WHATSAPP_CLOUD_MESSAGES_DOC_SOURCE: &str =
    "https://developers.facebook.com/docs/whatsapp/cloud-api/reference/messages";
pub const WHATSAPP_CLOUD_WEBHOOK_DOC_SOURCE: &str =
    "https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/components";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappAcceptedDeliveryEvent {
    pub schema_version: u16,
    pub task_id: String,
    pub response_digest: String,
    pub provider_message_ids: Vec<String>,
    pub accepted_at_ts: u64,
}

impl WhatsappAcceptedDeliveryEvent {
    pub fn validate(&self) -> bool {
        self.schema_version == WHATSAPP_ACCEPTED_DELIVERY_EVENT_SCHEMA_VERSION
            && !self.task_id.is_empty()
            && self.task_id.len() <= 128
            && self
                .task_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && self.response_digest.len() == 64
            && self
                .response_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && !self.provider_message_ids.is_empty()
            && self.provider_message_ids.len() <= 128
            && self.provider_message_ids.iter().all(|id| {
                !id.is_empty()
                    && id.len() <= 512
                    && !id
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            })
            && self.accepted_at_ts > 0
    }

    pub fn delivery_id(&self) -> String {
        format!(
            "delivery:{}:whatsapp-daemon:{}",
            self.task_id,
            &self.response_digest[..16]
        )
    }

    pub fn idempotency_key(&self) -> String {
        format!(
            "whatsapp-cloud:{}:daemon:{}",
            self.task_id, self.response_digest
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SendMessageResponse {
    #[serde(default)]
    messages: Vec<SendMessageRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SendMessageRef {
    #[serde(default)]
    id: String,
}

pub fn decode_message_ids(
    operation: &str,
    response_body: &str,
) -> Result<Vec<String>, ChannelProviderError> {
    let decoded = serde_json::from_str::<SendMessageResponse>(response_body).map_err(|_| {
        ChannelProviderError::invalid_response(
            WHATSAPP_CLOUD_SOURCE_ADAPTER,
            operation,
            response_body,
        )
    })?;
    let ids = decoded
        .messages
        .into_iter()
        .map(|message| message.id.trim().to_string())
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(ChannelProviderError::invalid_response(
            WHATSAPP_CLOUD_SOURCE_ADAPTER,
            operation,
            response_body,
        ));
    }
    Ok(ids)
}

pub fn customer_service_window_expires_at(last_inbound_at_ts: u64) -> u64 {
    last_inbound_at_ts.saturating_add(WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS)
}

pub fn customer_service_window_is_open(last_inbound_at_ts: u64, now_ts: u64) -> bool {
    last_inbound_at_ts > 0 && now_ts <= customer_service_window_expires_at(last_inbound_at_ts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsappTemplatePolicy {
    pub name: String,
    pub language: String,
}

impl WhatsappTemplatePolicy {
    pub fn from_config(name: &str, language: &str) -> Option<Self> {
        let name = name.trim();
        let language = language.trim();
        if !valid_template_token(name, 512) || !valid_template_token(language, 32) {
            return None;
        }
        Some(Self {
            name: name.to_string(),
            language: language.to_string(),
        })
    }
}

fn valid_template_token(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappWebhookPayload {
    #[serde(default)]
    pub entry: Vec<WhatsappWebhookEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappWebhookEntry {
    #[serde(default)]
    pub changes: Vec<WhatsappWebhookChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappWebhookChange {
    #[serde(default)]
    pub value: WhatsappWebhookValue,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappWebhookValue {
    #[serde(default)]
    pub statuses: Vec<WhatsappWebhookStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappWebhookStatus {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, deserialize_with = "deserialize_timestamp")]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub recipient_id: String,
    #[serde(default)]
    pub errors: Vec<WhatsappWebhookError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsappWebhookError {
    pub code: u64,
}

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Timestamp {
        Number(u64),
        String(String),
    }
    let value = Option::<Timestamp>::deserialize(deserializer)?;
    Ok(match value {
        Some(Timestamp::Number(value)) => Some(value),
        Some(Timestamp::String(value)) => value.parse::<u64>().ok(),
        None => None,
    })
}

impl WhatsappWebhookPayload {
    pub fn statuses(&self) -> impl Iterator<Item = &WhatsappWebhookStatus> {
        self.entry
            .iter()
            .flat_map(|entry| entry.changes.iter())
            .flat_map(|change| change.value.statuses.iter())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsappDeliveryEventStatus {
    Accepted,
    Delivered,
    Read,
    Failed,
}

impl WhatsappDeliveryEventStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Failed => "failed",
        }
    }
}

impl WhatsappWebhookStatus {
    pub fn delivery_status(&self) -> Option<WhatsappDeliveryEventStatus> {
        match self.status.trim().to_ascii_lowercase().as_str() {
            "sent" => Some(WhatsappDeliveryEventStatus::Accepted),
            "delivered" => Some(WhatsappDeliveryEventStatus::Delivered),
            "read" => Some(WhatsappDeliveryEventStatus::Read),
            "failed" | "deleted" => Some(WhatsappDeliveryEventStatus::Failed),
            _ => None,
        }
    }

    pub fn provider_error_code(&self) -> Option<String> {
        self.errors.first().map(|error| error.code.to_string())
    }
}

pub fn provider_error_from_response(
    operation: &str,
    status_code: u16,
    response_body: &str,
) -> ChannelProviderError {
    let provider_code = extract_error_code(response_body);
    let Some(code) = provider_code else {
        return ChannelProviderError::from_http_response(
            WHATSAPP_CLOUD_SOURCE_ADAPTER,
            operation,
            status_code,
            response_body,
        );
    };
    let failure_class = classify_error_code(code, status_code);
    ChannelProviderError::from_machine_failure(
        WHATSAPP_CLOUD_SOURCE_ADAPTER,
        operation,
        failure_class,
        Some(status_code),
        Some(&code.to_string()),
        None,
        response_body,
    )
}

fn extract_error_code(response_body: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(response_body).ok()?;
    value.pointer("/error/code").and_then(|code| {
        code.as_u64()
            .or_else(|| code.as_str().and_then(|value| value.parse().ok()))
    })
}

fn classify_error_code(code: u64, status_code: u16) -> ChannelProviderFailureClass {
    match code {
        190 => ChannelProviderFailureClass::Authentication,
        10 | 200 | 131005 | 131031 | 131042 => ChannelProviderFailureClass::PermissionDenied,
        130429 | 131048 | 131056 => ChannelProviderFailureClass::RateLimited,
        131021 | 131026 => ChannelProviderFailureClass::RecipientBlocked,
        131008 | 131009 | 131045 | 131047 | 131052 | 131053 => {
            ChannelProviderFailureClass::PayloadRejected
        }
        _ if status_code == 401 => ChannelProviderFailureClass::Authentication,
        _ if status_code == 403 => ChannelProviderFailureClass::PermissionDenied,
        _ if status_code == 429 => ChannelProviderFailureClass::RateLimited,
        _ if status_code >= 500 => ChannelProviderFailureClass::ProviderUnavailable,
        _ if status_code >= 400 => ChannelProviderFailureClass::PayloadRejected,
        _ => ChannelProviderFailureClass::Unknown,
    }
}

#[cfg(test)]
#[path = "channel_whatsapp_cloud_tests.rs"]
mod tests;
