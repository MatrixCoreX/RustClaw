use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelContentPart {
    Text {
        text: String,
    },
    Image {
        source: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
    ToolCall {
        call: ModelToolCall,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        provider_metadata: BTreeMap<String, Value>,
    },
    ToolResult {
        tool_call_id: String,
        content: Value,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelMessage {
    pub role: ModelRole,
    pub content: Vec<ModelContentPart>,
}

impl ModelMessage {
    pub fn text(role: ModelRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ModelContentPart::Text { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelToolChoice {
    #[default]
    Auto,
    Required,
}

impl ModelToolChoice {
    fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelTurnRequest {
    pub messages: Vec<ModelMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ModelToolDefinition>,
    #[serde(default, skip_serializing_if = "ModelToolChoice::is_auto")]
    pub tool_choice: ModelToolChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl ModelTurnRequest {
    pub fn text(prompt: impl Into<String>) -> Self {
        Self {
            messages: vec![ModelMessage::text(ModelRole::User, prompt)],
            tools: Vec::new(),
            tool_choice: ModelToolChoice::Auto,
            response_schema: None,
            stream: false,
            metadata: BTreeMap::new(),
        }
    }

    pub fn requires_native_tools(&self) -> bool {
        !self.tools.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelTurnUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Cancelled,
    Error,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelTurnEvent {
    Started {
        attempt: usize,
    },
    TextDelta {
        text: String,
    },
    ToolCallDelta {
        index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        arguments_delta: String,
    },
    ToolCall {
        call: ModelToolCall,
    },
    Usage {
        usage: ModelTurnUsage,
    },
    Finished {
        reason: ModelFinishReason,
    },
    Interrupted {
        code: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelTurnResponse {
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelTurnUsage>,
    pub finish_reason: ModelFinishReason,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reasoning_metadata: BTreeMap<String, Value>,
    #[serde(default)]
    pub events: Vec<ModelTurnEvent>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderModelCapabilities {
    pub native_tools: bool,
    pub parallel_tools: bool,
    pub structured_output: bool,
    pub streaming: bool,
    pub reasoning: bool,
    pub vision: bool,
    pub prompt_cache: bool,
}

#[cfg(test)]
#[path = "model_turn_tests.rs"]
mod tests;
