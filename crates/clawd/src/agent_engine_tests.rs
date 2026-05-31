use super::*;
use claw_core::skill_registry::{
    Capability, OutputKind, PlannerCapabilityEffect, PlannerCapabilityKind,
    PlannerCapabilityMapping, SkillKind, SkillManifest,
};
use serde_json::json;
use std::collections::BTreeMap;

fn test_skill_manifest(planner_capabilities: Vec<PlannerCapabilityMapping>) -> SkillManifest {
    SkillManifest {
        name: "fs_basic".to_string(),
        kind: SkillKind::Builtin,
        planner_kind: PlannerCapabilityKind::Tool,
        output_kind: OutputKind::Text,
        description: None,
        semantic_tags: Vec::new(),
        preferred_over_run_cmd: true,
        validation_actions: Vec::new(),
        prompt_file: None,
        input_schema: None,
        output_schema: None,
        runtime_skill: None,
        runtime_action: None,
        runtime_default_args: None,
        runtime_rewrite_arg_keys: Vec::new(),
        runtime_rewrite_semantic_kinds: Vec::new(),
        risk_level: None,
        auto_invocable: None,
        requires_confirmation: None,
        side_effect: None,
        confirmation_exempt_when: Vec::<BTreeMap<String, serde_json::Value>>::new(),
        timeout_seconds: None,
        retryable: None,
        group: None,
        primary_fallback_role: None,
        supported_os: Vec::new(),
        required_bins: Vec::new(),
        optional_bins: Vec::new(),
        platform_notes: Vec::new(),
        planner_capabilities,
        capabilities: vec![Capability::FsRead],
    }
}

#[test]
fn quick_index_includes_planner_capability_metadata() {
    let manifest = test_skill_manifest(vec![PlannerCapabilityMapping {
        name: "filesystem.list_entries".to_string(),
        action: Some("list_dir".to_string()),
        effect: Some(PlannerCapabilityEffect::Observe),
        required: vec!["path".to_string()],
        optional: vec!["limit".to_string()],
        risk_level: None,
        preferred: true,
    }]);

    let text = quick_index_planner_capabilities(&manifest);

    assert!(text.contains("planner_capabilities: filesystem.list_entries"));
    assert!(text.contains("action=list_dir"));
    assert!(text.contains("effect=observe"));
    assert!(text.contains("required=path"));
}

// --- build_safe_skill_args_summary: progress hint args must be whitelisted and safe ---
#[test]
fn test_build_safe_skill_args_summary_whitelist_order() {
    let args = json!({
        "symbol": "DOGEUSDT",
        "action": "open_orders",
        "exchange": "binance"
    });
    let s = build_safe_skill_args_summary(&args, 160);
    assert!(s.contains("action=open_orders"));
    assert!(s.contains("exchange=binance"));
    assert!(s.contains("symbol=DOGEUSDT"));
    assert!(s.starts_with("action="));
}

#[test]
fn test_build_safe_skill_args_summary_sensitive_omitted() {
    let args = json!({
        "action": "trade_submit",
        "symbol": "DOGEUSDT",
        "api_key": "secret123",
        "api_secret": "never-show"
    });
    let s = build_safe_skill_args_summary(&args, 160);
    assert!(!s.contains("api_key"));
    assert!(!s.contains("api_secret"));
    assert!(!s.contains("secret123"));
    assert!(s.contains("action=trade_submit"));
    assert!(s.contains("symbol=DOGEUSDT"));
}

#[test]
fn test_build_safe_skill_args_summary_truncate() {
    let args = json!({
        "action": "trade_history",
        "symbol": "DOGEUSDT",
        "limit": 10
    });
    let s = build_safe_skill_args_summary(&args, 30);
    assert!(s.len() <= 33);
    assert!(s.ends_with("...") || s.len() <= 30);
}

#[test]
fn test_build_safe_skill_args_summary_empty_object() {
    let args = json!({});
    let s = build_safe_skill_args_summary(&args, 160);
    assert!(s.is_empty());
}

#[test]
fn turn_analysis_prompt_block_includes_contract_matrix_for_structured_route() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        },
    };

    let block = build_turn_analysis_prompt_block(None, Some(&route));

    assert!(block.contains("- task_contract"));
    assert!(block.contains("- contract_matrix"));
    assert!(block.contains("required_evidence=candidates"));
    assert!(block.contains("final_answer_shape=name_list"));
    assert!(block.contains("allowed_actions="));
    assert!(block.contains("fs_basic"));
    assert!(block.contains("forbidden_actions="));
}

#[test]
fn register_step_output_indexes_inventory_names_for_followup_paths() {
    let mut loop_state = LoopState::new(1);
    register_step_output(
        &mut loop_state,
        1,
        1,
        "step_1",
        r#"{"action":"inventory_dir","names":["act_plan.log","clawd.log","clawd.run.log"],"path":"logs"}"#,
    );

    assert_eq!(
        loop_state
            .output_vars
            .get("last_output.0")
            .map(String::as_str),
        Some("act_plan.log")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("last_output.1")
            .map(String::as_str),
        Some("clawd.log")
    );
    assert_eq!(
        loop_state.output_vars.get("s1.names.2").map(String::as_str),
        Some("clawd.run.log")
    );
    assert_eq!(
        loop_state.output_vars.get("step_1[2]").map(String::as_str),
        Some("clawd.run.log")
    );
}

// --- build_final_delivery_with_priority: last_respond has priority over delivery_messages ---
#[test]
fn test_final_delivery_last_respond_priority_when_different() {
    let delivery = vec!["early answer".to_string()];
    let last_respond = "final answer".to_string();
    let (deduped, final_text, used) =
        crate::finalize::build_final_delivery_with_priority(&delivery, Some(&last_respond));
    assert!(used);
    assert_eq!(deduped.len(), 2);
    assert_eq!(deduped[0], "early answer");
    assert_eq!(deduped[1], "final answer");
    assert_eq!(final_text, "final answer");
}

#[test]
fn test_final_delivery_last_respond_same_as_delivery_no_duplicate() {
    let delivery = vec!["same text".to_string()];
    let last_respond = "same text".to_string();
    let (deduped, final_text, used) =
        crate::finalize::build_final_delivery_with_priority(&delivery, Some(&last_respond));
    assert!(used);
    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0], "same text");
    assert_eq!(final_text, "same text");
}

#[test]
fn test_final_delivery_no_last_respond_uses_delivery() {
    let delivery = vec!["only delivery".to_string()];
    let (deduped, final_text, used) =
        crate::finalize::build_final_delivery_with_priority(&delivery, None);
    assert!(!used);
    assert_eq!(deduped.len(), 1);
    assert_eq!(final_text, "only delivery");
}

#[test]
fn test_final_delivery_both_empty() {
    let delivery: Vec<String> = vec![];
    let (deduped, final_text, used) =
        crate::finalize::build_final_delivery_with_priority(&delivery, None);
    assert!(!used);
    assert!(deduped.is_empty());
    assert!(final_text.is_empty());
}

#[test]
fn test_final_delivery_strips_subtask_prefix_from_user_visible_messages() {
    let delivery = vec!["subtask#1 skill(run_cmd): success\ntestuser".to_string()];
    let (deduped, final_text, used) =
        crate::finalize::build_final_delivery_with_priority(&delivery, None);
    assert!(!used);
    assert_eq!(deduped, vec!["testuser".to_string()]);
    assert_eq!(final_text, "testuser");
}

#[test]
fn test_normalize_user_visible_text_strips_inline_subtask_prefix() {
    assert_eq!(
        crate::finalize::normalize_user_visible_text("subtask#1 skill(run_cmd): success testuser",),
        "testuser"
    );
}

#[test]
fn test_final_delivery_preserves_failed_message_body() {
    let delivery = vec!["subtask#1 skill(run_cmd): failed\npermission denied".to_string()];
    let (deduped, final_text, used) =
        crate::finalize::build_final_delivery_with_priority(&delivery, None);
    assert!(!used);
    assert_eq!(deduped, vec!["permission denied".to_string()]);
    assert_eq!(final_text, "permission denied");
}

#[test]
fn test_normalize_user_visible_text_strips_inline_failed_prefix() {
    assert_eq!(
        crate::finalize::normalize_user_visible_text(
            "subtask#1 skill(run_cmd): failed permission denied"
        ),
        "permission denied"
    );
}

#[test]
fn test_extract_latest_successful_read_file_output_prefers_content_body() {
    let mut loop_state = LoopState::default();
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "subtask#2".to_string(),
            skill: "read_file".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("# Test Workspace\nThis directory is reserved.".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    let observed = super::observed_output::extract_latest_successful_read_file_output(&loop_state);
    assert_eq!(
        observed.as_deref(),
        Some("# Test Workspace\nThis directory is reserved.")
    );
}

#[test]
fn test_extract_latest_successful_list_dir_output_prefers_content_body() {
    let mut loop_state = LoopState::default();
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "subtask#1".to_string(),
            skill: "list_dir".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("file1.txt\nsubdir/\nfile2.md".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    let observed = super::observed_output::extract_latest_successful_list_dir_output(&loop_state);
    assert_eq!(observed.as_deref(), Some("file1.txt\nsubdir/\nfile2.md"));
}

#[test]
fn test_extract_latest_generic_successful_output_prefers_non_read_non_list_skill() {
    let mut loop_state = LoopState::default();
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "subtask#1".to_string(),
            skill: "read_file".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("hello".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "subtask#2".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("testuser".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    let observed = super::observed_output::extract_latest_generic_successful_output(&loop_state)
        .expect("generic observed output");
    assert!(observed.action_label.contains("skill(run_cmd): success"));
    assert_eq!(observed.body, "testuser");
}

#[test]
fn test_extract_latest_generic_successful_output_skips_non_content() {
    let mut loop_state = LoopState::default();
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "subtask#1".to_string(),
            skill: "chat".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("FILE:/tmp/demo.txt".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    assert!(
        super::observed_output::extract_latest_generic_successful_output(&loop_state).is_none()
    );
}

#[test]
fn test_normalized_observed_listing_trims_blank_lines() {
    let observed = "\n file1.txt \n\n subdir/ \n";
    let out = super::observed_output::normalized_observed_listing(observed);
    assert_eq!(out.as_deref(), Some("file1.txt\nsubdir/"));
}

#[test]
fn test_finalizer_schema_answer_parse_ok() {
    let raw = r#"{"answer":"hello","completion_ok":true,"grounded_ok":true,"format_ok":true,"needs_clarify":false,"confidence":0.9,"used_evidence_ids":["E1"]}"#;
    let (answer, schema) = crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
    assert_eq!(answer, "hello");
    assert!(crate::finalize::finalizer_contract_ok(&schema));
}

#[test]
fn test_finalizer_schema_answer_parse_fail_non_json() {
    assert!(crate::finalize::finalizer_schema_answer("plain text").is_none());
}

#[test]
fn test_finalizer_contract_not_ok_when_grounding_false() {
    let raw = r#"{"answer":"hello","completion_ok":true,"grounded_ok":false,"format_ok":true}"#;
    let (_answer, schema) = crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
    assert!(!crate::finalize::finalizer_contract_ok(&schema));
    assert!(matches!(
        crate::finalize::finalizer_contract_disposition(&schema),
        crate::finalize::FinalizerDisposition::MustFail
    ));
}

#[test]
fn test_finalizer_contract_disposition_allows_fallback_on_format_only_failure() {
    let raw = r#"{"answer":"hello","completion_ok":true,"grounded_ok":true,"format_ok":false}"#;
    let (_answer, schema) = crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
    assert!(matches!(
        crate::finalize::finalizer_contract_disposition(&schema),
        crate::finalize::FinalizerDisposition::AllowFallback
    ));
}

#[test]
fn test_finalizer_contract_disposition_must_fail_on_needs_clarify() {
    let raw = r#"{"answer":"need info","completion_ok":false,"grounded_ok":true,"format_ok":true,"needs_clarify":true}"#;
    let (_answer, schema) = crate::finalize::finalizer_schema_answer(raw).expect("schema parse");
    assert!(matches!(
        crate::finalize::finalizer_contract_disposition(&schema),
        crate::finalize::FinalizerDisposition::MustFail
    ));
}

#[test]
fn test_internal_trace_artifact_detected() {
    assert!(crate::finalize::looks_like_internal_trace_artifact(
        "subtask#1 skill(run_cmd): success"
    ));
}

#[test]
fn test_structured_blob_detected() {
    assert!(crate::finalize::looks_like_structured_blob(
        "{\"answer\":\"x\"}"
    ));
    assert!(crate::finalize::looks_like_structured_blob("[1,2,3]"));
    assert!(!crate::finalize::looks_like_structured_blob("plain text"));
}

#[test]
fn test_infer_file_target_kind_classifies_extension_backed_files() {
    assert_eq!(
        crate::finalize::infer_file_target_kind("/tmp/app.log"),
        crate::finalize::FileTargetKind::LogFile
    );
    assert_eq!(
        crate::finalize::infer_file_target_kind("/tmp/data.json"),
        crate::finalize::FileTargetKind::JsonFile
    );
    assert_eq!(
        crate::finalize::infer_file_target_kind("/tmp/archive.tar.gz"),
        crate::finalize::FileTargetKind::ArchiveFile
    );
}

#[test]
fn test_infer_file_target_kind_distinguishes_directory_from_plain_file() {
    assert_eq!(
        crate::finalize::infer_file_target_kind("/tmp/output"),
        crate::finalize::FileTargetKind::Directory
    );
    assert_eq!(
        crate::finalize::infer_file_target_kind("/tmp/output.txt"),
        crate::finalize::FileTargetKind::File
    );
}

#[test]
fn test_parse_delivery_token_normalizes_supported_prefixes() {
    let (kind, payload) =
        crate::finalize::parse_delivery_token(" IMAGE_FILE:/tmp/demo.png ").expect("token");
    assert_eq!(kind, crate::finalize::DeliveryTokenKind::ImageFile);
    assert_eq!(payload.trim(), "/tmp/demo.png");
    assert_eq!(kind.canonical_prefix(), "FILE:");

    let (kind, payload) =
        crate::finalize::parse_delivery_token("MEDIA_URL:https://example.com/a.mp4")
            .expect("token");
    assert_eq!(kind, crate::finalize::DeliveryTokenKind::MediaUrl);
    assert_eq!(payload.trim(), "https://example.com/a.mp4");
}

#[test]
fn test_classify_planner_artifact_detects_tool_call_and_action_json() {
    assert!(matches!(
        crate::finalize::classify_planner_artifact("[TOOL_CALL]run_cmd[/TOOL_CALL]"),
        Some(crate::finalize::PlannerArtifactKind::ToolCallTag)
    ));
    assert!(matches!(
        crate::finalize::classify_planner_artifact(r#"{"type":"call_tool","tool":"read_file"}"#),
        Some(
            crate::finalize::PlannerArtifactKind::ActionObject
                | crate::finalize::PlannerArtifactKind::PlannerObject
        )
    ));
}

#[test]
fn test_user_safe_step_error_preserves_sanitized_error_excerpt() {
    assert_eq!(
        user_safe_step_error(
            "synthesize_answer could not produce a grounded publishable answer",
            false,
        ),
        "synthesize_answer could not produce a grounded publishable answer"
    );
    assert_eq!(
        user_safe_step_error("unknown action: read; allowed: info|inventory_dir", true),
        "unknown action: read; allowed: info|inventory_dir"
    );
    assert_eq!(
        user_safe_step_error("", false),
        "执行失败，但没有返回明确原因"
    );
    assert_eq!(
        user_safe_step_error("  ", true),
        "Execution failed without a clear reason."
    );
}

#[test]
fn test_extract_single_explicit_path_from_request_ok() {
    let text = "先读 /home/guagua/test/README.md 开头，再用一句话总结";
    assert_eq!(
        crate::finalize::extract_single_explicit_path_from_request(text).as_deref(),
        Some("/home/guagua/test/README.md")
    );
}

#[test]
fn test_observed_quotes_grounded_requires_exact_match() {
    let observed = "# Test Workspace\nThis directory is reserved for NL regression test artifacts.";
    let schema = crate::finalize::FinalizerSchemaOut {
        answer: "summary".to_string(),
        completion_ok: true,
        grounded_ok: true,
        format_ok: true,
        needs_clarify: false,
        confidence: 0.8,
        used_evidence_ids: vec!["E1".to_string()],
        evidence_quotes: vec!["NL regression test artifacts".to_string()],
    };
    assert!(crate::finalize::observed_quotes_grounded(&schema, observed));

    let bad = crate::finalize::FinalizerSchemaOut {
        evidence_quotes: vec!["high-performance distributed scheduler".to_string()],
        ..schema
    };
    assert!(!crate::finalize::observed_quotes_grounded(&bad, observed));
}

#[test]
fn test_observed_read_path_matches_request() {
    let ws = Path::new("/tmp/workspace");
    let user_text = "Read /home/guagua/test/README.md and summarize.";
    assert!(crate::finalize::observed_read_path_matches_request(
        ws,
        user_text,
        Some("/home/guagua/test/README.md")
    ));
    assert!(!crate::finalize::observed_read_path_matches_request(
        ws,
        user_text,
        Some("/home/guagua/rustclaw/README.md")
    ));
}
