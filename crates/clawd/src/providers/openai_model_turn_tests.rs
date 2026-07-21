use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use claw_core::model_turn::{ModelMessage, ModelRole, ModelToolDefinition, ModelTurnRequest};

use super::*;

fn provider() -> LlmProviderRuntime {
    LlmProviderRuntime {
        config: LlmProviderConfig {
            name: "vendor-minimax".to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            api_key: "test".to_string(),
            model: "MiniMax-M3".to_string(),
            context_window_tokens: Some(1_000_000),
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

fn native_request() -> ModelTurnRequest {
    ModelTurnRequest {
        messages: vec![ModelMessage::text(ModelRole::User, "inspect README.md")],
        tools: vec![ModelToolDefinition {
            name: "call_capability".to_string(),
            description: "Resolve and execute a runtime capability.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["capability", "args"],
                "properties": {
                    "capability": {"type": "string"},
                    "args": {"type": "object"}
                },
                "additionalProperties": false
            }),
            strict: true,
        }],
        response_schema: None,
        stream: false,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn native_request_maps_messages_and_function_tools() {
    let body = build_openai_request(&provider(), &native_request(), &ChatRequestHints::default())
        .expect("build request");

    assert_eq!(body["model"], "MiniMax-M3");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["tools"][0]["function"]["name"], "call_capability");
    assert_eq!(
        body["tools"][0]["function"]["parameters"]["required"],
        json!(["capability", "args"])
    );
    assert_eq!(body["parallel_tool_calls"], true);
}

#[test]
fn response_maps_parallel_tool_calls_without_prose_parsing() {
    let response = json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [
                    {
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "call_capability",
                            "arguments": "{\"capability\":\"fs.read\",\"args\":{\"path\":\"README.md\"}}"
                        }
                    },
                    {
                        "id": "call-2",
                        "type": "function",
                        "function": {
                            "name": "call_capability",
                            "arguments": "{\"capability\":\"fs.list\",\"args\":{\"path\":\"docs\"}}"
                        }
                    }
                ]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 8,
            "total_tokens": 28
        }
    });

    let turn = parse_openai_model_turn(&response).expect("parse turn");

    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[0].arguments["capability"], "fs.read");
    assert_eq!(turn.finish_reason, ModelFinishReason::ToolCalls);
    assert_eq!(turn.usage.and_then(|usage| usage.total_tokens), Some(28));
}

#[test]
fn malformed_tool_arguments_are_preserved_for_planner_contract_repair() {
    let response = json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "call_capability",
                        "arguments": "{not-json"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let turn = parse_openai_model_turn(&response).expect("transport preserves malformed call");
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "call_capability");
    assert_eq!(
        turn.tool_calls[0].arguments,
        Value::String("{not-json".to_string())
    );
    assert_eq!(turn.finish_reason, ModelFinishReason::ToolCalls);
}

#[test]
fn malformed_streamed_tool_arguments_are_preserved_for_planner_contract_repair() {
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    let frames = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call-1",
                        "function": {
                            "name": "call_capability",
                            "arguments": "{not-json"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
    );
    for frame in decoder.push(frames.as_bytes()).expect("decode stream") {
        accumulator.apply(frame, None).expect("apply stream");
    }

    let output = accumulator
        .finish(None)
        .expect("preserve malformed stream call");
    assert_eq!(output.turn.tool_calls.len(), 1);
    assert_eq!(
        output.turn.tool_calls[0].arguments,
        Value::String("{not-json".to_string())
    );
}

#[test]
fn sse_decoder_assembles_split_parallel_calls_and_usage() {
    let first_frame = format!(
        "data: {}\n\n",
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call-1",
                        "function": {
                            "name": "call_capability",
                            "arguments": "{\"capability\":\"fs.read\",\"args\":{"
                        }
                    }]
                }
            }]
        })
    );
    let split_at = first_frame.len() / 2;
    let chunks = vec![
        first_frame.as_bytes()[..split_at].to_vec(),
        first_frame.as_bytes()[split_at..].to_vec(),
        format!(
            "data: {}\r\n\r\n",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 1,
                            "id": "call-2",
                            "function": {
                                "name": "call_capability",
                                "arguments": "{\"capability\":\"fs.list\",\"args\":{}}"
                            }
                        }]
                    }
                }]
            })
        )
        .into_bytes(),
        format!(
            "data: {}\n\n",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": {"arguments": "}}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 6,
                    "total_tokens": 18
                }
            })
        )
        .into_bytes(),
        b"data: [DONE]\n\n".to_vec(),
    ];
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    for chunk in chunks {
        for frame in decoder.push(&chunk).expect("decode SSE frame") {
            accumulator.apply(frame, None).expect("apply SSE frame");
        }
    }

    let output = accumulator.finish(None).expect("finish stream");

    assert_eq!(output.turn.tool_calls.len(), 2);
    assert_eq!(output.turn.tool_calls[0].arguments["capability"], "fs.read");
    assert_eq!(output.turn.tool_calls[1].arguments["capability"], "fs.list");
    assert_eq!(
        output.turn.usage.and_then(|usage| usage.total_tokens),
        Some(18)
    );
    assert!(output
        .turn
        .events
        .iter()
        .any(|event| matches!(event, ModelTurnEvent::ToolCallDelta { index: 0, .. })));
}

#[test]
fn sse_disconnect_before_done_is_retryable_machine_condition() {
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    for frame in decoder
        .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")
        .expect("decode partial")
    {
        accumulator.apply(frame, None).expect("apply partial");
    }

    assert_eq!(
        accumulator.finish(None).expect_err("missing done rejected"),
        "model_turn_stream_disconnected"
    );
}

#[test]
fn sse_terminal_finish_reason_allows_eof_without_done_marker() {
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    for frame in decoder
        .push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"complete\"},\"finish_reason\":\"stop\"}]}\n\n",
        )
        .expect("decode terminal frame")
    {
        accumulator.apply(frame, None).expect("apply terminal frame");
    }
    accumulator.complete_terminal_eof();

    let output = accumulator.finish(None).expect("terminal finish accepted");

    assert_eq!(output.turn.text, "complete");
    assert_eq!(output.turn.finish_reason, ModelFinishReason::Stop);
}

#[test]
fn hidden_reasoning_delta_is_not_collected_or_emitted() {
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    for frame in decoder
        .push(
            b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"private\",\"content\":\"public\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        )
        .expect("decode frame")
    {
        accumulator.apply(frame, None).expect("apply frame");
    }

    let output = accumulator.finish(None).expect("finish stream");

    assert_eq!(output.turn.text, "public");
    assert!(output.turn.reasoning_metadata.is_empty());
    assert!(!output.raw_response.contains("private"));
}

#[test]
fn minimax_ndjson_stream_completes_on_terminal_finish_reason() {
    let chunks = [
        format!(
            "{}\n",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call-1",
                            "function": {
                                "name": "call_capability",
                                "arguments": "{\"capability\":\"fs_basic.read_text_range\","
                            }
                        }]
                    }
                }]
            })
        ),
        format!(
            "{}\n",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": {
                                "arguments": "\"args\":{\"path\":\"README.md\"}}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })
        ),
    ];
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    for chunk in chunks {
        for frame in decoder.push(chunk.as_bytes()).expect("decode NDJSON") {
            accumulator.apply(frame, None).expect("apply NDJSON");
        }
    }
    for frame in decoder.finish().expect("flush NDJSON") {
        accumulator.apply(frame, None).expect("apply tail");
    }
    accumulator.complete_terminal_eof();

    let output = accumulator.finish(None).expect("finish MiniMax stream");

    assert_eq!(output.turn.tool_calls.len(), 1);
    assert_eq!(
        output.turn.tool_calls[0].arguments["capability"],
        "fs_basic.read_text_range"
    );
    assert_eq!(output.turn.finish_reason, ModelFinishReason::ToolCalls);
}

#[test]
fn minimax_ndjson_without_finish_reason_remains_disconnected() {
    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    for frame in decoder
        .push(b"{\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n")
        .expect("decode NDJSON")
    {
        accumulator.apply(frame, None).expect("apply NDJSON");
    }
    accumulator.complete_terminal_eof();

    assert_eq!(
        accumulator
            .finish(None)
            .expect_err("unterminated NDJSON rejected"),
        "model_turn_stream_disconnected"
    );
}

#[test]
fn stream_raw_response_removes_minimax_think_content_across_frames() {
    let frames = vec![
        json!({"choices": [{"delta": {"content": "<think>private"}}]}),
        json!({"choices": [{"delta": {"content": " reasoning</think>public"}}]}),
        json!({"choices": [{"delta": {"content": " answer"}, "finish_reason": "stop"}]}),
    ];

    let raw =
        provider_safe_stream_raw_response(&frames, "<think>private reasoning</think>public answer");

    assert!(!raw.contains("private"));
    assert!(!raw.contains("<think>"));
    assert!(raw.contains("public answer"));
}
