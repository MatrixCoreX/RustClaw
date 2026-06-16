use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    config::{ModelProvider, RelayConfig},
    ApiError,
};

#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: Option<bool>,
}

impl ChatCompletionRequest {
    pub fn validate(&self, config: &RelayConfig) -> Result<(), ApiError> {
        if self.messages.is_empty() {
            return Err(ApiError::bad_request(
                "empty_messages",
                "proxy.empty_messages",
            ));
        }
        if self.stream.unwrap_or(false) {
            return Err(ApiError::bad_request(
                "stream_not_supported",
                "proxy.stream_not_supported",
            ));
        }
        if let Some(max_tokens) = self.max_tokens {
            if max_tokens > config.limits.max_tokens_per_request {
                return Err(ApiError::too_many_requests(
                    "max_tokens_exceeded",
                    "proxy.max_tokens_exceeded",
                ));
            }
        }
        if config.select_provider(self.model.as_deref()).is_none() {
            return Err(ApiError::bad_request(
                "model_not_allowed",
                "proxy.model_not_allowed",
            ));
        }
        Ok(())
    }

    pub fn to_upstream_body(&self, provider: &ModelProvider) -> Value {
        let mut body = Map::new();
        body.insert("model".to_owned(), json!(provider.model));
        body.insert("messages".to_owned(), json!(self.messages));
        body.insert("stream".to_owned(), json!(false));

        if let Some(temperature) = self.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(top_p) = self.top_p {
            body.insert("top_p".to_owned(), json!(top_p));
        }
        if let Some(max_tokens) = self.max_tokens {
            body.insert("max_tokens".to_owned(), json!(max_tokens));
        }

        Value::Object(body)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message_key: &'static str,
    #[serde(rename = "type")]
    pub error_type: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ModelList {
    pub object: &'static str,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: &'static str,
}

impl ModelList {
    pub fn from_providers(providers: &[ModelProvider]) -> Self {
        Self {
            object: "list",
            data: providers
                .iter()
                .map(|provider| ModelInfo {
                    id: provider.alias.clone(),
                    object: "model",
                    created: Utc::now().timestamp(),
                    owned_by: "llm-relay-server",
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

pub fn extract_usage(body: &Value) -> Usage {
    let Some(usage) = body.get("usage") else {
        return Usage::default();
    };

    let prompt_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);

    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    }
}

pub fn mask_model_name(body: &mut Value, public_model: &str) {
    if let Some(object) = body.as_object_mut() {
        object.insert("model".to_owned(), json!(public_model));
    }
}
