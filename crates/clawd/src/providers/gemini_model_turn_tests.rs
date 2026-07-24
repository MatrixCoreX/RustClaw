use std::collections::BTreeMap;
use std::sync::Arc;

use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use claw_core::model_turn::{
    ModelContentPart, ModelFinishReason, ModelMessage, ModelRole, ModelToolCall, ModelToolChoice,
    ModelToolDefinition, ModelTurnRequest,
};
use serde_json::json;

use super::{build_gemini_request, parse_gemini_model_turn};

fn provider() -> crate::LlmProviderRuntime {
    crate::LlmProviderRuntime {
        config: LlmProviderConfig {
            name: "vendor-google".to_string(),
            provider_type: "google_gemini".to_string(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            api_key: "test".to_string(),
            model: "gemini-3.1-pro-preview".to_string(),
            context_window_tokens: None,
            input_modalities: vec!["text".to_string(), "image".to_string()],
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
fn request_maps_system_function_declarations_signatures_and_results() {
    let mut metadata = BTreeMap::new();
    metadata.insert("thoughtSignature".to_string(), json!("opaque-signature"));
    let request = ModelTurnRequest {
        messages: vec![
            ModelMessage::text(ModelRole::System, "system contract"),
            ModelMessage::text(ModelRole::User, "inspect"),
            ModelMessage {
                role: ModelRole::Assistant,
                content: vec![ModelContentPart::ToolCall {
                    call: ModelToolCall {
                        id: "call-1".to_string(),
                        name: "read_file".to_string(),
                        arguments: json!({"path": "README.md"}),
                    },
                    provider_metadata: metadata,
                }],
            },
            ModelMessage {
                role: ModelRole::Tool,
                content: vec![ModelContentPart::ToolResult {
                    tool_call_id: "call-1".to_string(),
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
        response_schema: Some(json!({
            "type": "object",
            "properties": {"ok": {"type": "boolean"}}
        })),
        stream: true,
        metadata: BTreeMap::new(),
    };

    let body =
        build_gemini_request(&provider(), &request, &Default::default()).expect("request mapping");

    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        "system contract"
    );
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["parameters"]["required"][0],
        "path"
    );
    assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
    assert_eq!(
        body["contents"][1]["parts"][0]["thoughtSignature"],
        "opaque-signature"
    );
    assert_eq!(
        body["contents"][2]["parts"][0]["functionResponse"]["name"],
        "read_file"
    );
    assert_eq!(
        body["generationConfig"]["responseMimeType"],
        "application/json"
    );
}

#[test]
fn response_parses_parallel_calls_usage_and_thought_signature() {
    let turn = parse_gemini_model_turn(&json!({
        "candidates": [{
            "content": {"role": "model", "parts": [
                {"text": "Checking."},
                {
                    "functionCall": {"id": "call-1", "name": "read_file", "args": {"path": "a"}},
                    "thoughtSignature": "opaque-signature"
                },
                {"functionCall": {"id": "call-2", "name": "read_file", "args": {"path": "b"}}}
            ]},
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 20,
            "candidatesTokenCount": 10,
            "totalTokenCount": 30
        }
    }))
    .expect("response mapping");

    assert_eq!(turn.text, "Checking.");
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.finish_reason, ModelFinishReason::ToolCalls);
    assert_eq!(
        turn.reasoning_metadata["tool_call_metadata"]["call-1"]["thoughtSignature"],
        "opaque-signature"
    );
    assert_eq!(
        turn.usage.as_ref().and_then(|usage| usage.total_tokens),
        Some(30)
    );
}

#[test]
fn tool_result_requires_a_matching_prior_tool_call() {
    let request = ModelTurnRequest {
        messages: vec![ModelMessage {
            role: ModelRole::Tool,
            content: vec![ModelContentPart::ToolResult {
                tool_call_id: "unknown".to_string(),
                content: json!("result"),
                is_error: false,
            }],
        }],
        tools: Vec::new(),
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        stream: false,
        metadata: BTreeMap::new(),
    };

    let error = build_gemini_request(&provider(), &request, &Default::default())
        .expect_err("unmatched result must fail");

    assert_eq!(error, "gemini_tool_result_name_missing index=0");
}
