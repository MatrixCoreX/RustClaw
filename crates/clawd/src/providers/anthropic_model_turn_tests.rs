use std::collections::BTreeMap;
use std::sync::Arc;

use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use claw_core::model_turn::{
    ModelContentPart, ModelFinishReason, ModelMessage, ModelRole, ModelToolCall, ModelToolChoice,
    ModelToolDefinition, ModelTurnRequest,
};
use serde_json::json;

use super::{build_anthropic_request, parse_anthropic_model_turn};

fn provider() -> crate::LlmProviderRuntime {
    crate::LlmProviderRuntime {
        config: LlmProviderConfig {
            name: "vendor-anthropic".to_string(),
            provider_type: "anthropic_claude".to_string(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key: "test".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            context_window_tokens: None,
            input_modalities: vec!["text".to_string()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 30,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        pricing: None,
        latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
        client: reqwest::Client::new(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    }
}

#[test]
fn request_maps_system_tools_parallel_calls_and_results() {
    let provider = provider();
    let call = ModelToolCall {
        id: "toolu_1".to_string(),
        name: "read_file".to_string(),
        arguments: json!({"path": "README.md"}),
    };
    let request = ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, "system contract"),
            ModelMessage::text(ModelRole::User, "inspect"),
            ModelMessage {
                role: ModelRole::Assistant,
                content: vec![ModelContentPart::ToolCall {
                    call,
                    provider_metadata: BTreeMap::new(),
                }],
            },
            ModelMessage {
                role: ModelRole::Tool,
                content: vec![ModelContentPart::ToolResult {
                    tool_call_id: "toolu_1".to_string(),
                    content: json!({"text": "ok"}),
                    is_error: false,
                }],
            },
        ],
        tools: vec![ModelToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
            strict: true,
        }],
        tool_choice: ModelToolChoice::Required,
        response_schema: None,
        stream: true,
        metadata: BTreeMap::new(),
    };

    let body =
        build_anthropic_request(&provider, &request, &Default::default()).expect("request mapping");

    assert_eq!(body["system"][0]["text"], "system contract");
    assert_eq!(body["tools"][0]["input_schema"]["required"][0], "path");
    assert_eq!(body["tools"][0]["strict"], true);
    assert_eq!(body["tool_choice"]["type"], "any");
    assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
    assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
    assert!(body.get("stream").is_none());
}

#[test]
fn response_parses_text_parallel_tool_calls_usage_and_finish_reason() {
    let turn = parse_anthropic_model_turn(&json!({
        "content": [
            {"type": "text", "text": "Checking."},
            {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "a"}},
            {"type": "tool_use", "id": "toolu_2", "name": "read_file", "input": {"path": "b"}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 12, "output_tokens": 8}
    }))
    .expect("response mapping");

    assert_eq!(turn.text, "Checking.");
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[1].arguments["path"], "b");
    assert_eq!(turn.finish_reason, ModelFinishReason::ToolCalls);
    assert_eq!(
        turn.usage.as_ref().and_then(|usage| usage.total_tokens),
        Some(20)
    );
    assert!(turn.events.len() >= 5);
}

#[test]
fn response_rejects_tool_use_without_structured_input() {
    let error = parse_anthropic_model_turn(&json!({
        "content": [{"type": "tool_use", "id": "toolu_1", "name": "read_file"}],
        "stop_reason": "tool_use"
    }))
    .expect_err("missing input must fail");

    assert_eq!(error, "anthropic_tool_call_input_missing index=0");
}
