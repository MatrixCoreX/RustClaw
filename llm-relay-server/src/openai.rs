use std::collections::HashSet;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    config::{ModelProvider, RelayConfig},
    ApiError,
};

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct ChatCompletionRequest(Value);

impl ChatCompletionRequest {
    pub fn validate(&self, config: &RelayConfig) -> Result<(), ApiError> {
        let object = self
            .0
            .as_object()
            .ok_or_else(|| ApiError::bad_request("invalid_request", "proxy.invalid_request"))?;
        let allowed = allowed_request_fields();
        if object.keys().any(|key| !allowed.contains(key.as_str())) {
            return Err(ApiError::bad_request(
                "unsupported_request_field",
                "proxy.unsupported_request_field",
            ));
        }

        let messages = object
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| ApiError::bad_request("invalid_messages", "proxy.invalid_messages"))?;
        if messages.is_empty() || messages.len() > config.max_messages {
            return Err(ApiError::bad_request(
                "message_count_out_of_range",
                "proxy.message_count_out_of_range",
            ));
        }
        if object
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() > config.max_tools)
        {
            return Err(ApiError::bad_request(
                "tool_count_out_of_range",
                "proxy.tool_count_out_of_range",
            ));
        }
        if object
            .get("stream")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(ApiError::bad_request(
                "invalid_stream_value",
                "proxy.invalid_stream_value",
            ));
        }
        if let Some(max_tokens) = self.max_tokens() {
            if max_tokens > config.limits.max_tokens_per_request {
                return Err(ApiError::too_many_requests(
                    "max_tokens_exceeded",
                    "proxy.max_tokens_exceeded",
                ));
            }
        }
        if config.select_provider(self.model()).is_none() {
            return Err(ApiError::bad_request(
                "model_not_allowed",
                "proxy.model_not_allowed",
            ));
        }
        Ok(())
    }

    pub fn model(&self) -> Option<&str> {
        self.0.get("model").and_then(Value::as_str)
    }

    pub fn is_streaming(&self) -> bool {
        self.0
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn max_tokens(&self) -> Option<u32> {
        self.0
            .get("max_completion_tokens")
            .or_else(|| self.0.get("max_tokens"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    }

    pub fn to_upstream_body(&self, provider: &ModelProvider) -> Value {
        let mut body = self.0.as_object().cloned().unwrap_or_else(Map::new);
        body.insert("model".to_owned(), json!(provider.model));
        Value::Object(body)
    }
}

fn allowed_request_fields() -> HashSet<&'static str> {
    [
        "model",
        "messages",
        "temperature",
        "top_p",
        "max_tokens",
        "max_completion_tokens",
        "stream",
        "stream_options",
        "stop",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "response_format",
        "reasoning_effort",
        "frequency_penalty",
        "presence_penalty",
        "seed",
        "user",
        "n",
        "logprobs",
        "top_logprobs",
    ]
    .into_iter()
    .collect()
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
    pub fn from_provider(provider: &ModelProvider) -> Self {
        Self {
            object: "list",
            data: vec![ModelInfo {
                id: provider.alias.clone(),
                object: "model",
                created: Utc::now().timestamp(),
                owned_by: "managed-relay",
            }],
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
