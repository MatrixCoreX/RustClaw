use super::{llm_trace_text_lines, task_llm_trace_url};

#[test]
fn llm_trace_request_explicitly_opts_into_teaching_trace() {
    let url = task_llm_trace_url("http://127.0.0.1:8787", "task-llm-trace")
        .expect("build teaching trace URL");

    assert_eq!(
        url.as_str(),
        "http://127.0.0.1:8787/v1/debug/tasks/task-llm-trace?teaching=true"
    );
}

#[test]
fn llm_trace_text_lines_number_calls_and_flow_tokens() {
    let debug = serde_json::json!({
        "task_id": "task-llm-trace",
        "goal_id": "goal-llm-trace",
        "user_id": 7,
        "chat_id": 9,
        "call_count": 2,
        "flow_summary": {
            "stage_count": 2,
            "retry_count": 0,
            "verifier_call_count": 1,
            "finalizer_call_count": 0,
            "provider_error_count": 0
        },
        "model_catalog_trace": {
            "readiness": {
                "schema_version": 1,
                "selected_provider": "minimax",
                "selected_model": "MiniMax-M3",
                "selected_entry_status": "found",
                "entry_count": 2,
                "matched_entry_count": 1,
                "credential_state": "configured_env",
                "ready": true,
                "text_generation": true,
                "image_input": true,
                "image_understanding": true,
                "image_generation": false,
                "audio_input": false,
                "audio_transcription": false,
                "audio_generation": false,
                "video_input": true,
                "video_generation": false,
                "music_generation": false,
                "async_required": false,
                "dry_run": false
            }
        },
        "calls": [
            {
                "call_index": 1,
                "flow": {
                    "prompt_label": "plan",
                    "flow_stage": "agent_loop.planner",
                    "flow_node": "planner_round",
                    "code_module": "crates/clawd/src/agent_engine/planning.rs",
                    "code_entrypoint": "plan_round_actions",
                    "trigger_kind": "normal"
                },
                "status": "ok",
                "vendor": "minimax",
                "provider": "minimax",
                "model": "MiniMax-M3",
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 7,
                    "total_tokens": 18
                }
            },
            {
                "call_index": 2,
                "flow": {
                    "prompt_label": "answer_verifier",
                    "flow_stage": "agent_loop.answer_verifier",
                    "flow_node": "answer_verifier",
                    "code_module": "crates/clawd/src/answer_verifier_runtime.rs",
                    "code_entrypoint": "verify_answer_observe_only",
                    "trigger_kind": "normal"
                },
                "status": "ok",
                "vendor": "minimax",
                "provider": "minimax",
                "model": "MiniMax-M3"
            }
        ]
    });

    let lines = llm_trace_text_lines(&debug, false, None);

    assert!(lines.contains(&"llm_trace_task_id: task-llm-trace".to_string()));
    assert!(lines.contains(&"llm_trace_goal_id: goal-llm-trace".to_string()));
    assert!(lines.contains(&"llm_trace_session_id=user_chat:7:9".to_string()));
    assert!(lines.contains(&"llm_trace_call_count: 2".to_string()));
    assert!(lines.contains(&"llm_trace_flow_stage_count: 2".to_string()));
    assert!(lines.iter().any(|line| {
        line.contains("llm_trace_model_readiness:")
            && line.contains("trace_ref=model_catalog_trace.readiness")
            && line.contains("selected_provider=minimax")
            && line.contains("selected_model=MiniMax-M3")
            && line.contains("selected_entry_status=found")
            && line.contains("matched_entry_count=1")
            && line.contains("credential_state=configured_env")
            && line.contains("ready=true")
            && line.contains("image_understanding=true")
            && line.contains("video_input=true")
            && line.contains("dry_run=false")
    }));
    assert!(lines.iter().any(|line| {
        line == "llm_trace_call: llm_call_ref=LLM#1 index=1 status=ok vendor=minimax provider=minimax model=MiniMax-M3 prompt_label=plan flow_stage=agent_loop.planner flow_node=planner_round code_module=crates/clawd/src/agent_engine/planning.rs code_entrypoint=plan_round_actions trigger_kind=normal prompt_tokens=11 completion_tokens=7 total_tokens=18"
    }));
    assert!(lines.iter().any(|line| {
        line.contains("llm_call_ref=LLM#2")
            && line.contains("index=2")
            && line.contains("prompt_label=answer_verifier")
            && line.contains("flow_stage=agent_loop.answer_verifier")
    }));
}

#[test]
fn llm_trace_text_lines_project_missing_model_readiness() {
    let debug = serde_json::json!({
        "task_id": "task-llm-missing-model",
        "call_count": 0,
        "model_catalog_trace": {
            "readiness": {
                "schema_version": 1,
                "selected_provider": "minimax",
                "selected_model": "missing-model",
                "selected_entry_status": "missing",
                "entry_count": 1,
                "matched_entry_count": 0,
                "credential_state": "null",
                "ready": false,
                "text_generation": false,
                "image_input": false,
                "image_understanding": false,
                "image_generation": false,
                "audio_input": false,
                "audio_transcription": false,
                "audio_generation": false,
                "video_input": false,
                "video_generation": false,
                "music_generation": false,
                "async_required": false,
                "dry_run": false
            }
        },
        "calls": []
    });

    let lines = llm_trace_text_lines(&debug, false, None);

    assert!(lines.iter().any(|line| {
        line.contains("llm_trace_model_readiness:")
            && line.contains("selected_model=missing-model")
            && line.contains("selected_entry_status=missing")
            && line.contains("matched_entry_count=0")
            && line.contains("credential_state=null")
            && line.contains("ready=false")
            && line.contains("text_generation=false")
    }));
}

#[test]
fn llm_trace_text_lines_limit_and_raw_fields() {
    let debug = serde_json::json!({
        "task_id": "task-llm-raw",
        "call_count": 2,
        "calls": [
            {
                "call_index": 1,
                "flow": {
                    "prompt_label": "normalizer",
                    "flow_stage": "boundary.normalizer",
                    "flow_node": "intent_normalizer",
                    "code_module": "normalizer.rs",
                    "code_entrypoint": "run_intent_normalizer_model_step",
                    "trigger_kind": "normal"
                },
                "status": "ok",
                "request_payload": {
                    "messages": [
                        {
                            "role": "user",
                            "content": "TRACE_INPUT_TOKEN"
                        }
                    ]
                },
                "response": "TRACE_RESPONSE_TOKEN",
                "raw_response": "{\"content\":\"TRACE_RAW_TOKEN\"}"
            },
            {
                "call_index": 2,
                "flow": {
                    "prompt_label": "plan",
                    "flow_stage": "agent_loop.planner",
                    "flow_node": "planner_round",
                    "code_module": "planning.rs",
                    "code_entrypoint": "plan_round_actions",
                    "trigger_kind": "normal"
                },
                "status": "ok",
                "response": "SHOULD_BE_LIMITED_OUT"
            }
        ]
    });

    let lines = llm_trace_text_lines(&debug, true, Some(1));

    assert!(lines.iter().any(|line| line.contains("llm_call_ref=LLM#1")));
    assert!(lines.iter().any(|line| line.contains("index=1")));
    assert!(!lines.iter().any(|line| line.contains("llm_call_ref=LLM#2")));
    assert!(!lines.iter().any(|line| line.contains("index=2")));
    assert!(lines.contains(&"llm_request_payload_1:".to_string()));
    assert!(lines.iter().any(|line| line.contains("TRACE_INPUT_TOKEN")));
    assert!(lines.contains(&"llm_response_1:".to_string()));
    assert!(lines.contains(&"TRACE_RESPONSE_TOKEN".to_string()));
    assert!(lines.contains(&"llm_raw_response_1:".to_string()));
    assert!(lines.contains(&"{\"content\":\"TRACE_RAW_TOKEN\"}".to_string()));
    assert!(!lines
        .iter()
        .any(|line| line.contains("SHOULD_BE_LIMITED_OUT")));
}
