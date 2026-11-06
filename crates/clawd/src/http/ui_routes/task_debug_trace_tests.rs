use super::{build_model_catalog_trace_for_debug, extract_resume_trace_for_debug, TaskDebugEntry};
use serde_json::json;
use std::path::PathBuf;

fn temp_debug_workspace_root() -> PathBuf {
    let path = std::env::temp_dir().join(format!("agent-debug-trace-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).expect("claim private temp workspace");
    path
}

fn write_minimax_debug_catalog(root: &PathBuf, selected_vendor: &str, selected_model: &str) {
    std::fs::create_dir_all(root.join("configs")).expect("configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        format!(
            r#"
[server]
request_timeout_seconds = 30
[database]
sqlite_path = 'data/fixture.db'
busy_timeout_ms = 2000
[worker]

[llm]
selected_vendor = "{selected_vendor}"
selected_model = "{selected_model}"

[llm.minimax]
api_format = "openai_compat"
base_url = "https://api.minimaxi.com/v1"
api_key = "catalog-secret"
model = "MiniMax-M3"
models = ["MiniMax-M3"]
input_modalities = ["text", "image", "video"]
output_modalities = ["text"]
context_window_tokens = 1000000
"#
        ),
    )
    .expect("write config");
    std::fs::write(
        root.join("configs/image.toml"),
        r#"
[image_vision]
minimax_models = ["MiniMax-M3"]
"#,
    )
    .expect("write image config");
    claw_core::secrets::model_environment::save(root, "minimax", "catalog-secret")
        .expect("write effective text credential");
}

#[test]
fn task_debug_model_catalog_trace_projects_secret_free_capabilities() {
    let root = temp_debug_workspace_root();
    write_minimax_debug_catalog(&root, "minimax", "MiniMax-M3");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;
    let entries = vec![TaskDebugEntry {
        ts: Some(10),
        task_id: Some("task-1".to_string()),
        parent_task_id: None,
        child_task_id: None,
        call_id: Some("task-1:planner".to_string()),
        vendor: Some("minimax".to_string()),
        provider: Some("vendor-minimax".to_string()),
        provider_type: Some("openai_compat".to_string()),
        model: Some("MiniMax-M3".to_string()),
        model_kind: None,
        status: Some("ok".to_string()),
        mode: None,
        prompt_source: Some("layered:prompts/lightweight_execution_prompt.md".to_string()),
        prompt_hash: None,
        prompt_file: None,
        prompt: None,
        request_payload: Some(json!({"messages": []})),
        response: None,
        raw_response: Some("{}".to_string()),
        clean_response: None,
        sanitized: None,
        error: None,
        usage: None,
    }];

    let trace = build_model_catalog_trace_for_debug(&state, &entries);

    assert_eq!(trace["trace_kind"], "model_catalog_decision");
    assert_eq!(trace["status"], "ok");
    assert_eq!(trace["selected_provider"], "minimax");
    assert_eq!(trace["selected_model"], "MiniMax-M3");
    assert_eq!(trace["observed_providers"][0], "minimax");
    assert_eq!(trace["entries"][0]["schema_version"], 2);
    assert_eq!(trace["entries"][0]["models"], json!(["MiniMax-M3"]));
    assert!(trace["entries"][0]
        .as_object()
        .is_some_and(|entry| entry.contains_key("timeout_seconds")));
    let actual_config = claw_core::config::AppConfig::load(
        &state
            .skill_rt
            .workspace_root
            .join("configs/config.toml")
            .to_string_lossy(),
    )
    .expect("complete runtime fixture");
    let actual_providers =
        crate::llm_gateway::build_providers_for_selection(&actual_config, None, None);
    assert_eq!(actual_providers.len(), 1);
    assert_eq!(
        trace["entries"][0]["timeout_seconds"],
        json!(actual_providers[0].config.timeout_seconds)
    );
    assert_eq!(
        trace["entries"][0]["input_modalities"],
        json!(["text", "image", "video"])
    );
    assert_eq!(trace["entries"][0]["output_modalities"], json!(["text"]));
    assert_eq!(trace["entries"][0]["supports_image_input"], true);
    assert_eq!(trace["entries"][0]["supports_image_understanding"], true);
    assert_eq!(trace["entries"][0]["supports_audio_transcription"], false);
    assert_eq!(trace["entries"][0]["active_text_provider"], true);
    assert_eq!(trace["entries"][0]["credential_state"], "configured_env");
    assert_eq!(trace["readiness"]["schema_version"], 2);
    assert_eq!(trace["readiness"]["selected_provider"], "minimax");
    assert_eq!(trace["readiness"]["selected_model"], "MiniMax-M3");
    assert_eq!(trace["readiness"]["selected_entry_status"], "found");
    assert_eq!(trace["readiness"]["entry_count"], 1);
    assert_eq!(trace["readiness"]["matched_entry_count"], 1);
    assert_eq!(trace["readiness"]["credential_state"], "configured_env");
    assert_eq!(trace["readiness"]["ready"], true);
    assert_eq!(trace["readiness"]["text_generation"], true);
    assert_eq!(trace["readiness"]["image_input"], true);
    assert_eq!(trace["readiness"]["image_understanding"], true);
    assert_eq!(trace["readiness"]["video_input"], true);
    assert_eq!(trace["readiness"]["async_required"], false);
    assert_eq!(trace["readiness"]["dry_run"], false);
    assert_eq!(trace["vendor_patch_names"][0], "minimax");
    assert!(!trace.to_string().contains("catalog-secret"));
}

#[test]
fn task_debug_model_catalog_trace_marks_missing_selected_model_not_ready() {
    let root = temp_debug_workspace_root();
    write_minimax_debug_catalog(&root, "fixture-missing-provider", "missing-model");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;
    let entries = vec![TaskDebugEntry {
        ts: Some(10),
        task_id: Some("task-1".to_string()),
        parent_task_id: None,
        child_task_id: None,
        call_id: Some("task-1:planner".to_string()),
        vendor: Some("minimax".to_string()),
        provider: Some("vendor-minimax".to_string()),
        provider_type: Some("openai_compat".to_string()),
        model: Some("MiniMax-M3".to_string()),
        model_kind: None,
        status: Some("ok".to_string()),
        mode: None,
        prompt_source: Some("layered:prompts/lightweight_execution_prompt.md".to_string()),
        prompt_hash: None,
        prompt_file: None,
        prompt: None,
        request_payload: Some(json!({"messages": []})),
        response: None,
        raw_response: Some("{}".to_string()),
        clean_response: None,
        sanitized: None,
        error: None,
        usage: None,
    }];

    let trace = build_model_catalog_trace_for_debug(&state, &entries);

    assert_eq!(trace["selected_model"], "missing-model");
    assert_eq!(trace["readiness"]["selected_model"], "missing-model");
    assert_eq!(trace["readiness"]["selected_entry_status"], "missing");
    assert_eq!(trace["readiness"]["matched_entry_count"], 0);
    assert_eq!(trace["readiness"]["credential_state"], "null");
    assert_eq!(trace["readiness"]["ready"], false);
    assert_eq!(trace["readiness"]["text_generation"], false);
    assert_eq!(trace["entries"][0]["model"], "MiniMax-M3");
    assert!(!trace.to_string().contains("catalog-secret"));
}

#[test]
fn task_debug_resume_trace_projects_checkpoint_machine_fields() {
    let result_json = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "provider_gap_retry_window",
            "next_check_after": 1781800300,
            "checkpoint_id": "ckpt-1"
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": "ckpt-1",
            "resume_entrypoint": "next_planner_round",
            "budget": {
                "round": 2,
                "step": 3,
                "llm_calls": 4,
                "tool_calls": 5,
                "elapsed_ms": 6000,
                "llm_elapsed_ms": 3000,
                "tool_elapsed_ms": 2000
            },
            "completed_side_effect_refs": ["write_file:tmp/a.txt"],
            "evidence_refs": ["step:evidence:1"],
            "artifact_refs": ["changed_file:tmp/a.txt"],
            "last_successful_round": 2,
            "repair_signal": {
                "status_code": "provider_rate_limited",
                "external_provider_blocked": true,
                "provider": "minimax",
                "retry_after_seconds": 30,
                "next_recovery_kind": "background_wait"
            }
        }
    });

    let trace = extract_resume_trace_for_debug("running", &result_json).expect("resume trace");

    assert_eq!(trace["trace_kind"], "task_resume_decision");
    assert_eq!(trace["state"], "waiting");
    assert_eq!(trace["checkpoint_id"], "ckpt-1");
    assert_eq!(trace["resume_entrypoint"], "next_planner_round");
    assert_eq!(trace["completed_side_effect_count"], 1);
    assert_eq!(trace["requires_idempotency_guard"], true);
    assert_eq!(trace["provider_blocker_active"], true);
    assert_eq!(
        trace["provider_blocker_status_code"],
        "provider_rate_limited"
    );
    assert_eq!(trace["task_checkpoint"]["checkpoint_id"], "ckpt-1");
}
