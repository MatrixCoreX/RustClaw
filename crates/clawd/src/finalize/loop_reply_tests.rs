use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use std::time::{SystemTime, UNIX_EPOCH};

use super::directory_purpose;
use super::{
    agent_context_allows_observed_output_language_fallback,
    append_compound_file_delivery_token_from_route,
    attach_config_edit_observed_answer_from_registry,
    attach_deterministic_observed_execution_status_answer,
    attach_execution_recipe_closeout_to_delivery, attach_execution_summary_to_delivery,
    auto_requested_success_marker, backfill_delivery_from_last_outputs,
    build_execution_summary_message, build_execution_summary_messages,
    build_pending_user_input_clarify_reason, compare_paths_size_ratio_answer,
    content_evidence_missing_target_answer, content_evidence_step_failure_answer,
    content_evidence_terminal_respond_is_contractual_answer,
    delivery_contract_suppresses_execution_summary, delivery_is_content_answer_candidate,
    deterministic_execution_failed_step_answer, deterministic_matrix_observed_shape_answer,
    deterministic_missing_observed_target_answer, deterministic_observed_execution_status_answer,
    deterministic_structured_file_validation_from_read_range, direct_config_edit_observed_answer,
    direct_current_workspace_top_level_dirs_overview_answer, direct_db_basic_observed_answer,
    direct_directory_purpose_summary_from_size_facts,
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_find_entries, direct_file_token_from_observed_inventory,
    direct_generated_file_path_report_from_dry_run_payload, direct_non_builtin_skill_raw_answer,
    direct_path_from_active_bound_inventory, direct_publishable_observed_answer,
    direct_quantity_comparison_from_compare_paths, direct_raw_command_output_projection,
    direct_rustclaw_config_risk_answer, direct_scalar_observed_answer,
    direct_structured_observed_answer,
    discard_non_answer_separator_delivery_for_broad_structured_read,
    discard_raw_passthrough_delivery_when_structured_answer_available,
    ensure_requested_success_marker_visible, execution_recipe_closeout_note,
    final_answer_text_from_delivery, finalize_loop_reply, finalizer_requires_clarify,
    generated_delivery_existing_file_content_synthesis_token, has_missing_file_search_evidence,
    language_rendered_failed_step_finalizer_summary,
    latest_delivery_preserves_observed_quantity_size_facts,
    latest_file_delivery_observation_is_missing,
    latest_path_batch_facts_has_implicit_metadata_fields, looks_like_raw_command_snapshot,
    looks_like_structured_machine_output, markdown_heading_from_read_output,
    matrix_strict_list_observed_answer, missing_file_path_from_output,
    missing_requested_success_marker, normalize_file_token_delivery_from_auto_locator,
    normalize_file_token_delivery_from_observed_paths,
    observed_delivery_has_complete_contract_evidence,
    observed_execution_without_publishable_delivery_outcome,
    observed_execution_without_publishable_delivery_reply, observed_synthesis_unavailable_reply,
    path_batch_size_comparison_answer, prefer_latest_synthesis_for_compound_observation_delivery,
    prefer_observed_answer_for_exact_contract, preferred_route_clarify_question,
    priority_last_respond_for_final_delivery, promote_observed_language_delivery_summary,
    replace_delivery_with_deterministic_current_workspace_dirs_overview_answer,
    replace_delivery_with_deterministic_directory_purpose_answer,
    replace_delivery_with_deterministic_execution_failed_step_answer,
    replace_delivery_with_deterministic_observed_execution_status_answer,
    replace_delivery_with_deterministic_quantity_comparison_answer,
    replace_delivery_with_deterministic_recent_artifacts_judgment_answer,
    replace_delivery_with_deterministic_rustclaw_config_risk_answer,
    replace_delivery_with_latest_tail_read_range_answer,
    replace_delivery_with_observed_markdown_heading_scalar,
    replace_delivery_with_requested_machine_kv_summary,
    replace_git_repository_state_delivery_with_requested_machine_fields,
    replace_raw_observation_delivery_with_synthesis, resolve_file_token_from_auto_locator_answer,
    route_prefers_language_rendered_execution_failed_step, route_structured_clarify_context,
    run_compatibility_fallback_renderer_registry, run_task_lifecycle_renderer_registry,
    should_attach_execution_summary, should_drop_passthrough_delivery_for_content_evidence,
    should_return_missing_file_delivery_reply, should_try_observed_output_language_fallback,
    structured_compound_synthesis_can_replace_current_delivery, structured_json_values_from_output,
    successful_delivery_final_status, verify_summary_requires_resume_confirmation,
    visible_answer_is_machine_payload, visible_machine_payload_should_remain_structured,
};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    AgentRuntimeConfig, AppState, ClaimedTask, IntentOutputContract, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

#[path = "loop_reply_tests/machine_kv_json_guard.rs"]
mod machine_kv_json_guard;

#[test]
fn visible_answer_machine_payload_detection_is_structural() {
    assert!(visible_answer_is_machine_payload(
        r#"{"message_key":"clawd.msg.config_edit.guard","candidates":["tools.allow_sudo=true"]}"#
    ));
    assert!(visible_answer_is_machine_payload(
        r#"{"contract_marker":"filesystem_mutation_result","status":"ok","steps":[{"action":"ingest","path":"README.md"}]}"#
    ));
    assert!(!visible_answer_is_machine_payload(
        "configs/config.toml has one observed risk."
    ));
}

#[test]
fn priority_last_respond_does_not_override_qualified_delivery_after_tool_observation() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state
        .delivery_messages
        .push("完整观察答案，包含日志分析、文档摘要和表格".to_string());
    loop_state.last_user_visible_respond = Some("| name | score |".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "transform".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("| name | score |".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        ..Default::default()
    };

    assert!(priority_last_respond_for_final_delivery(&loop_state, Some(&summary), false).is_none());
}

#[test]
fn priority_last_respond_does_not_override_delivery_when_summary_is_missing() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state
        .delivery_messages
        .push("完整观察答案，包含日志分析、文档摘要和表格".to_string());
    loop_state.last_user_visible_respond = Some("| name | score |".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "transform".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("| name | score |".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    assert!(priority_last_respond_for_final_delivery(&loop_state, None, false).is_none());
}

#[test]
fn priority_last_respond_keeps_explicit_respond_step_priority() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state
        .delivery_messages
        .push("older delivery".to_string());
    loop_state.last_user_visible_respond = Some("explicit respond answer".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_4".to_string(),
        skill: "respond".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("explicit respond answer".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        ..Default::default()
    };

    assert_eq!(
        priority_last_respond_for_final_delivery(&loop_state, Some(&summary), false)
            .map(String::as_str),
        Some("explicit respond answer")
    );
}

#[test]
fn config_guard_machine_payload_remains_structured_for_final_delivery() {
    assert!(visible_machine_payload_should_remain_structured(
        r#"{"message_key":"clawd.msg.config_edit.guard","path":"configs/config.toml","risk_count":2,"candidates":["tools.allow_sudo=true"]}"#
    ));
    assert!(visible_machine_payload_should_remain_structured(
        r#"{"message_key":"clawd.msg.config_risk.summary","path":"configs/config.toml","count":1,"risks":["tools.allow_sudo=true"]}"#
    ));
    assert!(!visible_machine_payload_should_remain_structured(
        r#"{"message_key":"clawd.msg.config_edit.guard","count":1}"#
    ));
}

#[test]
fn subagent_runtime_machine_payload_remains_structured_for_final_delivery() {
    assert!(visible_machine_payload_should_remain_structured(
        r#"{"output_format":"machine_json","owner_layer":"subagent_runtime","execution_mode":"bounded_parallel_readonly_child_runs","aggregation":{"finding_refs":[]}}"#
    ));
}

#[path = "loop_reply_execution_summary_tests.rs"]
mod execution_summary_tests;

#[path = "loop_reply_execution_summary_text_boundary_tests.rs"]
mod execution_summary_text_boundary_tests;

#[path = "loop_reply_exact_contract_tests.rs"]
mod exact_contract_tests;

#[path = "loop_reply_content_evidence_tests.rs"]
mod content_evidence_tests;

#[path = "loop_reply_directory_purpose_tests.rs"]
mod directory_purpose_tests;

#[path = "loop_reply_quantity_tests.rs"]
mod quantity_tests;

#[path = "loop_reply_language_closeout_tests.rs"]
mod language_closeout_tests;

#[path = "loop_reply_config_edit_tests.rs"]
mod config_edit_tests;

#[path = "loop_reply_missing_delivery_tests.rs"]
mod missing_delivery_tests;

#[path = "loop_reply_structured_observation_tests.rs"]
mod structured_observation_tests;

#[path = "loop_reply_execution_status_tests.rs"]
mod execution_status_tests;

#[path = "loop_reply_observed_contract_tests.rs"]
mod observed_contract_tests;

#[path = "loop_reply_raw_command_tests.rs"]
mod raw_command_tests;

#[path = "loop_reply_raw_command_text_boundary_tests.rs"]
mod raw_command_text_boundary_tests;

#[path = "loop_reply_service_status_tests.rs"]
mod service_status_tests;

#[path = "loop_reply_service_status_text_boundary_tests.rs"]
mod service_status_text_boundary_tests;

#[path = "loop_reply_error_finalize_tests.rs"]
mod error_finalize_tests;

#[path = "loop_reply_scalar_direct_tests.rs"]
mod scalar_direct_tests;

#[path = "loop_reply_file_delivery_tests.rs"]
mod file_delivery_tests;

#[path = "loop_reply_file_missing_tests.rs"]
mod file_missing_tests;

#[path = "loop_reply_filesystem_mutation_tests.rs"]
mod filesystem_mutation_tests;

#[path = "loop_reply_delivery_backfill_tests.rs"]
mod delivery_backfill_tests;

#[path = "loop_reply_content_evidence_passthrough_tests.rs"]
mod content_evidence_passthrough_tests;

#[path = "loop_reply_git_state_tests.rs"]
mod git_state_tests;

#[path = "loop_reply_markdown_scalar_tests.rs"]
mod markdown_scalar_tests;

#[path = "loop_reply_markdown_scalar_text_boundary_tests.rs"]
mod markdown_scalar_text_boundary_tests;

#[path = "loop_reply_matrix_shape_tests.rs"]
mod matrix_shape_tests;

#[path = "loop_reply_machine_envelope_tests.rs"]
mod machine_envelope_tests;

#[path = "loop_reply_machine_kv_text_boundary_tests.rs"]
mod machine_kv_text_boundary_tests;

#[path = "loop_reply_clarify_envelope_tests.rs"]
mod clarify_envelope_tests;

#[path = "loop_reply_task_lifecycle_renderers_tests.rs"]
mod task_lifecycle_renderers_tests;

#[path = "loop_reply_compatibility_renderers_tests.rs"]
mod compatibility_renderers_tests;

#[path = "loop_reply_capability_result_renderers_tests.rs"]
mod capability_result_renderers_tests;

#[path = "loop_reply_artifact_renderers_tests.rs"]
mod artifact_renderers_tests;

#[path = "loop_reply_final_answer_renderers_tests.rs"]
mod final_answer_renderers_tests;

#[path = "loop_reply_route_helpers_tests.rs"]
mod route_helpers_tests;

#[path = "loop_reply_tail_read_tests.rs"]
mod tail_read_tests;

#[path = "loop_reply_weather_tests.rs"]
mod weather_tests;

#[test]
fn requested_machine_kv_summary_replaces_raw_observed_delivery() {
    let task = claimed_task("task-machine-kv-summary-finalizer");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        &serde_json::json!({
            "extra": {
                "action": "read_range",
                "path": "AGENTS.md",
                "excerpt": "248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes"
            },
            "text": "{\"action\":\"read_range\",\"excerpt\":\"248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes\"}"
        })
        .to_string(),
    ));
    let mut delivery_messages = vec![
        "248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes"
            .to_string(),
    ];
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Use read_range only. Answer exactly as machine summary: required=yes script=check_runtime_semantic_rewrite_boundary.py.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["required=yes script=check_runtime_semantic_rewrite_boundary.py"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("required=yes script=check_runtime_semantic_rewrite_boundary.py")
    );
    assert_eq!(
        finalizer_summary
            .as_ref()
            .and_then(|summary| summary.grounded_ok),
        Some(true)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_richer_recent_scalar_delivery() {
    let task = claimed_task("task-machine-kv-summary-recent-scalar-richer");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"same_path":false},"field_value":{"left_exists":true,"right_exists":true,"same_path":false},"left":{"exists":true},"right":{"exists":true}}}"#,
    ));
    let mut delivery_messages =
        vec!["same_path=false\nleft_exists=true\nright_exists=true".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut route = free_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return same_path and existence fields",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["same_path=false\nleft_exists=true\nright_exists=true".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("same_path=false\nleft_exists=true\nright_exists=true")
    );
}

#[test]
fn requested_machine_kv_summary_preserves_richer_required_evidence_delivery() {
    let task = claimed_task("task-machine-kv-summary-required-evidence-richer");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","facts":[{"path":"service_notes.md","exists":true},{"path":"release_checklist.md","exists":true}]}}"#,
    ));
    let mut delivery_messages = vec![
        "same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true"
            .to_string(),
    ];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut route = free_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    let _ = replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return same_path and both exist fields",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true"
                .to_string()
        ]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true")
    );
}

#[test]
fn hook_policy_surface_json_can_replace_short_token_delivery() {
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","path":"configs/agent_guard.toml"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"plan/current.md","excerpt":"Track I"}"#,
    ));
    let synthesis = serde_json::json!({
        "message_key": "clawd.msg.agent_hooks.pre_tool_use_policy_surface",
        "reason_code": "agent_hooks_pre_tool_use_policy_surface",
        "owner_layer": "agent_hooks",
        "stage": "pre_tool_use",
        "decision_tokens": ["allow", "deny", "require_confirmation", "background_wait"],
        "decisions": {
            "allow": {"supported": true},
            "deny": {"supported": true},
            "require_confirmation": {"supported": true},
            "background_wait": {"supported": true}
        }
    })
    .to_string();

    assert!(structured_compound_synthesis_can_replace_current_delivery(
        &route,
        &loop_state,
        "require_confirmation background_wait stage=pre_tool_use",
        &synthesis,
    ));
}

#[test]
fn grounded_compound_delivery_preserves_latest_terminal_language_over_observed_projection() {
    let task = claimed_task("task-grounded-compound-terminal");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "log_analyze",
        r#"{"keyword_counts":{"error":1,"warn":2},"path":"logs/app.log"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r##"{"action":"read_range","path":"docs/service_notes.md","excerpt":"# Service Notes\nrestart guidance"}"##,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "transform",
        r#"{"formatted":"| name | score |\n| beta | 12 |"}"#,
    ));
    let terminal_answer = concat!(
        "1) log evidence: error=1 warn=2\n",
        "2) document evidence: Service Notes restart guidance\n",
        "3) table:\n",
        "| name | score |\n",
        "| beta | 12 |"
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "synthesize_answer",
        terminal_answer,
    ));
    let mut delivery_messages = vec!["| name | score |\n| beta | 12 |".to_string()];
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = delivery_messages.first().cloned();
    let mut finalizer_summary = None;

    assert!(prefer_latest_synthesis_for_compound_observation_delivery(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    ));

    assert_eq!(delivery_messages, vec![terminal_answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(terminal_answer)
    );
    assert_eq!(
        finalizer_summary.as_ref().and_then(|summary| summary.stage),
        Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_publishable_summary_over_marker_only_summary() {
    let task = claimed_task("task-machine-kv-marker-only-summary");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        "fs_basic planner_kind",
    ));
    let table = "| 检查项 | 结果 |\n|---|---|\n| README.md 是否存在 | 是 |\n| docs 文件名 | release_checklist.md、service_notes.md |\n| logs 直接子项数量 | 2 |\n| fs_basic 的 planner_kind | tool |";
    let tagged_table = format!("markdown\n{table}");
    let mut delivery_messages = vec![tagged_table.clone()];
    loop_state.last_user_visible_respond = Some(tagged_table.clone());
    loop_state.last_publishable_synthesis_output = Some(tagged_table);
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "检查 README.md、列出 docs 文件名、统计 logs 直接子项数量，并读取 fs_basic 的 planner_kind，最后用表格回答。",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![table.to_string()]);
    assert_eq!(loop_state.last_user_visible_respond.as_deref(), Some(table));
}

#[test]
fn requested_machine_kv_summary_preserves_structured_media_dry_run_projection() {
    let task = claimed_task("task-machine-kv-media-dry-run-projection");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let output_path = "/home/guagua/rustclaw/document/media_dry_run/image_status_card.png";
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "image_generate",
        &serde_json::json!({
            "text": "IMAGE_GENERATE_DRY_RUN",
            "extra": {
                "dry_run": true,
                "provider": "minimax",
                "model": "image-01",
                "model_kind": "dry_run",
                "output_path": output_path,
                "planned_outputs": [{
                    "path": output_path,
                    "type": "image_file"
                }]
            }
        })
        .to_string(),
    ));
    let current = concat!(
        "dry_run=true\n",
        "provider=minimax\n",
        "model=image-01\n",
        "model_kind=dry_run\n",
        "output_path=/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\n",
        "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\",\"type\":\"image_file\"}]"
    );
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "use image.generate dry_run=true and return provider/model planned_outputs output_path",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(current)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_async_cancel_adapter_projection() {
    let task = claimed_task("task-machine-kv-async-cancel-projection");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "image_generate",
        r#"{"text":"IMAGE_CANCEL_DRY_RUN","extra":{"async_cancel_adapter_result":{"adapter_kind":"media_job_poll","job_id":"image-job-001","status":"cancelled","cancellation_result_json":{"task_id":"image-task-001","job_id":"image-job-001","status":"cancelled","dry_run":true}}}}"#,
    ));
    let current = concat!(
        "task_id=image-task-001\n",
        "job_id=image-job-001\n",
        "status=cancelled\n",
        "async_cancel_adapter_result={\"adapter_kind\":\"media_job_poll\",\"job_id\":\"image-job-001\",\"status\":\"cancelled\"}"
    );
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return task_id job_id cancelled status and async_cancel_adapter_result",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(current)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_publishable_command_summary() {
    let task = claimed_task("task-machine-kv-summary-command-summary");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        r#"{"extra":{"action":"run_cmd","command":"pwd","command_output":"/home/guagua/rustclaw"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "process_basic",
        r#"{"extra":{"action":"port_list","port":8787,"process":"clawd","pid":892143}}"#,
    ));
    let full_answer = "The working directory is /home/guagua/rustclaw. A clawd-related process is running, and port 8787 is visible.";
    let mut delivery_messages = vec![full_answer.to_string()];
    loop_state.last_user_visible_respond = Some(full_answer.to_string());
    let mut route = free_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "Run pwd, inspect the local port, and answer with the working directory and whether a port is visible."
                .to_string(),
        ),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Run pwd, inspect the local port, and answer with the working directory and whether a port is visible.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![full_answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(full_answer)
    );
    assert_ne!(delivery_messages, vec!["port=8787".to_string()]);
}

#[test]
fn requested_machine_kv_summary_preserves_agent_hook_policy_surface_delivery() {
    let task = claimed_task("task-machine-kv-agent-hook-surface");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_fields","path":"configs/agent_guard.toml","results":[{"field_path":"agent.hooks.blocked_action_refs","value":[]},{"field_path":"agent.hooks.blocked_tools","value":[]},{"field_path":"agent.hooks.require_confirmation_action_refs","value":[]},{"field_path":"agent.hooks.background_wait_action_refs","value":[]}]}}"#,
    ));
    let full_answer = "stage=pre_tool_use\nagent.hooks.blocked_action_refs=[]\nagent.hooks.blocked_tools=[]\nagent.hooks.require_confirmation_action_refs=[]\nagent.hooks.background_wait_action_refs=[]";
    let mut delivery_messages = vec![full_answer.to_string()];
    loop_state.last_user_visible_respond = Some(full_answer.to_string());
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "最终输出必须包含机器字段 stage=pre_tool_use",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![full_answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(full_answer)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_web_search_listing_delivery() {
    let task = claimed_task("task-machine-kv-summary-web-search-listing");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "web_search_extract",
        r#"{"extra":{"action":"search_extract","top_k":3,"candidates":[{"title":"tdejager/tutorial_bot","source":"github.com","url":"https://github.com/tdejager/tutorial_bot"},{"title":"volodymyrd/rust-async-tutorial","source":"github.com","url":"https://github.com/volodymyrd/rust-async-tutorial"}]}}"#,
    ));
    let answer = "tdejager/tutorial_bot\nvolodymyrd/rust-async-tutorial".to_string();
    let mut delivery_messages = vec![answer.clone()];
    loop_state.last_user_visible_respond = Some(answer.clone());
    let mut route = free_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "capability_ref=web.search_results top_k=3".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Search the web for Rust async tutorial top_k=3 and return titles only.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![answer.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer.as_str())
    );
}

#[test]
fn requested_machine_kv_summary_restores_service_status_terminal_delivery() {
    let task = claimed_task("task-machine-kv-summary-service-status-terminal");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "docker_basic",
        r#"{"extra":{"action":"version","available":false,"command_succeeded":false,"output":"docker unavailable: No such file or directory (os error 2)"},"text":"docker unavailable: No such file or directory (os error 2)"}"#,
    ));
    let terminal = "Docker version (read-only check)\n- status: unavailable\n- source: docker_basic (action=version)\n- command_succeeded: false\n- field_value: unavailable";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", terminal));
    let mut delivery_messages = vec!["docker.version".to_string()];
    loop_state.last_user_visible_respond = Some("docker.version".to_string());
    let mut route = free_route_result();
    route.resolved_intent = "docker.version".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("docker.version".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Check Docker version read-only.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![terminal.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(terminal)
    );
}

#[test]
fn requested_machine_kv_summary_restores_service_capability_terminal_delivery_without_semantic_kind(
) {
    let task = claimed_task("task-machine-kv-summary-service-capability-terminal");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "service_control",
        r#"{"extra":{"action":"status","target":"clawd","status":"ok","manager_type":"rustclaw","verified":true},"text":"{}"}"#,
    ));
    let terminal = "target=clawd\nstatus=ok\nmanager_type=rustclaw\nverified=true";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", terminal));
    let mut delivery_messages = vec!["service.status".to_string()];
    loop_state.last_user_visible_respond = Some("service.status".to_string());
    let mut route = free_route_result();
    route.route_reason = "capability_ref=service.status".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("service.status".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Check clawd service status.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![terminal.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(terminal)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_service_control_observed_field_projection() {
    let task = claimed_task("task-machine-kv-preserve-service-control-observed-fields");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "service_control",
        r#"{"extra":{"service_name":"telegramd","target":"telegramd","post_state":"telegramd=running","pre_state":"telegramd=running","status":"ok","verified":true,"manager_type":"rustclaw","summary":"Status: telegramd=running"}}"#,
    ));
    let current = concat!(
        "target=telegramd service_name=telegramd post_state=telegramd=running ",
        "pre_state=telegramd=running status=ok verified=true manager_type=rustclaw ",
        "source=service_control"
    )
    .to_string();
    let mut delivery_messages = vec![current.clone()];
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(current.clone());
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "check whether telegramd is running right now and briefly explain the status",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(current.as_str())
    );
}

#[test]
fn requested_machine_kv_summary_preserves_colon_field_value_delivery() {
    let task = claimed_task("task-machine-kv-summary-colon-fields");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|Archive fixtures for NL tests.\n2|This subdirectory exists so the docs directory has a nested child for directory-count and names-only prompts.","path":"/tmp/README.txt"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|Archive fixtures for NL tests.\\n2|This subdirectory exists so the docs directory has a nested child for directory-count and names-only prompts.\",\"path\":\"/tmp/README.txt\"}"}"#,
    ));
    let answer =
        "text_excerpt: \"Archive fixtures for NL tests.\"\ndetected_format: plain text (.txt)";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", answer));
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.last_user_visible_respond = Some(answer.to_string());
    let mut delivery_messages = vec![answer.to_string()];
    let mut route = free_route_result();
    route.resolved_intent = "text_excerpt detected_format".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return text_excerpt and detected_format.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
}

#[test]
fn requested_machine_kv_summary_requires_observed_non_flag_value() {
    let task = claimed_task("task-machine-kv-summary-finalizer-missing");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"extra":{"action":"read_range","excerpt":"248|must run another_guard.py"}}"#,
    ));
    let mut delivery_messages = vec!["248|must run another_guard.py".to_string()];
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Answer exactly as machine summary: required=yes script=missing_guard.py.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec!["248|must run another_guard.py"]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn requested_machine_kv_summary_uses_state_patch_required_field() {
    let task = claimed_task("task-machine-kv-summary-state-patch");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.last_user_visible_respond = Some(
        "After boundary changes, run `python3 scripts/check_runtime_semantic_rewrite_boundary.py`."
            .to_string(),
    );
    let mut delivery_messages = Vec::new();
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_field": "required=yes script=check_runtime_semantic_rewrite_boundary.py"
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Read AGENTS.md lines 248-249.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["required=yes script=check_runtime_semantic_rewrite_boundary.py"]
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["required=yes script=check_runtime_semantic_rewrite_boundary.py"]
    );
}

#[test]
fn requested_machine_kv_summary_replaces_prose_when_state_patch_requires_machine_fields() {
    let task = claimed_task("task-machine-kv-strict-state-patch-prose");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "git_basic",
        r#"{"extra":{"action":"repository_state","branch":"main","remotes":["origin","backup"]}}"#,
    ));
    let current = "Current repository state: branch=main, remotes include origin and backup.";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_machine_fields": ["branch", "remotes"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return repository machine fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![r#"branch=main remotes=["origin","backup"]"#]
    );
}

#[test]
fn requested_machine_kv_summary_replaces_partial_machine_delivery_for_required_fields() {
    let task = claimed_task("task-machine-kv-strict-state-patch-partial");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","changed_count":2,"paths":["tmp/a.txt","tmp/b.txt"]}}"#,
    ));
    let mut delivery_messages = vec!["changed_count=2".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_machine_fields": ["changed_count", "paths"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return mutation machine fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![r#"changed_count=2 paths=["tmp/a.txt","tmp/b.txt"]"#]
    );
}

#[test]
fn requested_machine_kv_summary_projects_git_status_fields_from_user_request() {
    let task = claimed_task("task-git-status-machine-kv-user-request");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "git_basic",
        r#"{"extra":{"action":"status","branch":"main","changed_count":0,"field_value":{"branch":"main","changed_count":0,"paths":[],"worktree_state":"clean"},"paths":[],"worktree_state":"clean"},"text":"exit=0\n## main...origin/main\n"}"#,
    ));
    let current = "状态检查已完成，但还需要重新整理字段。";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "只返回 branch、worktree_state、changed_count 三个字段。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["branch=main worktree_state=clean changed_count=0"]
    );
}

#[test]
fn requested_machine_kv_summary_overrides_scalar_path_when_explicit_pair_is_observed() {
    let task = claimed_task("task-machine-kv-explicit-pair-over-scalar-path");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","matches":[{"line":242,"path":"AGENTS.md","text":"run `python3 scripts/check_no_nl_hardmatch.py` after boundary changes"}],"query":"check_no_nl_hardmatch.py","results":["AGENTS.md"],"root":"AGENTS.md"},"text":"AGENTS.md"}"#,
    ));
    let current = "AGENTS.md";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Only keep no_hardmatch_guard=check_no_nl_hardmatch.py.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["no_hardmatch_guard=check_no_nl_hardmatch.py"]
    );
}

#[test]
fn requested_machine_kv_summary_projects_empty_git_paths() {
    let task = claimed_task("task-git-status-empty-paths");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "git_basic",
        r#"{"extra":{"action":"status","changed_count":0,"field_value":{"changed_count":0,"paths":[]},"paths":[]},"text":"exit=0\n## main...origin/main\n"}"#,
    ));
    let mut delivery_messages = vec!["exit=0 command=git status --porcelain".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "只返回 changed_count 和 paths。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![r#"changed_count=0 paths=[]"#]);
}

#[test]
fn requested_machine_kv_summary_replaces_conflicting_machine_values_for_required_field() {
    let task = claimed_task("task-machine-kv-strict-conflicting-values");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","contains_rustclaw":true}}"#,
    ));
    let mut delivery_messages = vec!["contains_rustclaw=true contains_rustclaw=false".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_machine_fields": ["contains_rustclaw"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return content check machine fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec!["contains_rustclaw=true"]);
}

#[test]
fn requested_machine_kv_summary_patches_empty_machine_field_in_rich_answer() {
    let task = claimed_task("task-machine-kv-patch-empty-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let current =
        "clawcli resume is available.\n\nFields:\n- <TASK_ID>\n- --text <TEXT>\n\nresume_task_id=";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;
    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![
            "clawcli resume is available.\n\nFields:\n- <TASK_ID>\n- --text <TEXT>\n\nresume_task_id=<TASK_ID>"
        ]
    );
}

#[test]
fn requested_machine_kv_summary_patches_none_machine_field_in_rich_answer() {
    let task = claimed_task("task-machine-kv-patch-none-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let current = "clawcli resume is available.\n\nresume_task_id=<none>";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["clawcli resume is available.\n\nresume_task_id=<TASK_ID>"]
    );
}

#[test]
fn requested_machine_kv_summary_preserves_rich_answer_with_requested_machine_line() {
    let task = claimed_task("task-machine-kv-preserve-rich-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let current = "clawcli resume is available.\n\nFields:\n- <TASK_ID>\n- --text <TEXT>\n\nresume_task_id=<TASK_ID>";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.to_string()]);
}

#[test]
fn requested_machine_kv_summary_preserves_latest_rich_answer_over_stale_machine_value() {
    let task = claimed_task("task-machine-kv-preserve-latest-rich-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let latest = "clawcli resume is available.\n\nFields:\n- task_id: <TASK_ID>\n- text: <TEXT>\n\nresume_task_id=<TASK_ID>";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", latest));
    loop_state.last_user_visible_respond = Some(latest.to_string());
    let mut delivery_messages = vec![
        "resume_task_id=null".to_string(),
        "resume_task_id=not_applicable".to_string(),
    ];
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![latest.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(latest)
    );
}

#[test]
fn requested_machine_kv_summary_ignores_context_summary_machine_tokens() {
    let task = claimed_task("task-machine-kv-context-token");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", "false"));
    let mut delivery_messages = vec!["false".to_string()];
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        context_bundle_summary: Some(
            "current_workspace_scope_from_current_request=false".to_string(),
        ),
        ..Default::default()
    };

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return the async timeout policy fields",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec!["false"]);
}

#[test]
fn requested_machine_kv_summary_preserves_full_structured_contract_json() {
    let task = claimed_task("task-machine-kv-structured-contract");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let contract = serde_json::json!({
        "schema_version": 1,
        "contract_marker": "async_job_poll_contract_dry_run",
        "adapter_result": {"type": "pending_async_job"},
        "async_timeout_policy": {"effective_deadline_ts": "min(deadline_ts,max_runtime_deadline_ts)"}
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", &contract));
    loop_state.delivery_messages.push(contract.clone());
    loop_state.last_user_visible_respond = Some(contract.clone());
    let mut delivery_messages = vec![contract.clone()];
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "current_workspace_scope_from_current_request=false",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![contract]);
}

#[test]
fn requested_machine_kv_summary_preserves_config_guard_machine_payload() {
    let task = claimed_task("task-machine-kv-config-guard");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let guard = serde_json::json!({
        "message_key": "clawd.msg.config_edit.guard",
        "reason_code": "config_edit_guard_risk_found",
        "path": "/home/guagua/rustclaw/configs/config.toml",
        "count": 2,
        "risk_count": 2,
        "candidates": [
            "tools.allow_sudo=true",
            "tools.allow_path_outside_workspace=true"
        ],
        "enabled": false
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "config_basic", &guard));
    loop_state.delivery_messages.push(guard.clone());
    loop_state.last_user_visible_respond = Some(guard.clone());
    let mut delivery_messages = vec![guard.clone()];
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "检查 configs/config.toml 是否有明显空字段或禁用技能数量，不要输出任何 secret、token、key 的值。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![guard]);
}

#[test]
fn requested_machine_kv_summary_restores_config_guard_payload_for_summary_route() {
    let task = claimed_task("task-machine-kv-config-guard-restore");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let guard = serde_json::json!({
        "message_key": "clawd.msg.config_edit.guard",
        "reason_code": "config_edit_guard_risk_found",
        "path": "/home/guagua/rustclaw/configs/config.toml",
        "risk_count": 2,
        "candidates": [
            "tools.allow_sudo=true",
            "tools.allow_path_outside_workspace=true"
        ],
        "risks": [
            "tools.allow_sudo=true",
            "tools.allow_path_outside_workspace=true"
        ]
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "config_basic", &guard));
    let mut delivery_messages = vec!["count=2".to_string()];
    let mut finalizer_summary = None;
    let mut route = free_route_result();
    route.resolved_intent =
        "Inspect configs/config.toml and report disabled skill count plus empty fields."
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "检查 configs/config.toml 是否有明显空字段或禁用技能数量，不要输出任何 secret、token、key 的值。",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![guard.clone()]);
    assert_eq!(loop_state.delivery_messages, vec![guard]);
}

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_loop_finalize_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn claimed_task(task_id: &str) -> ClaimedTask {
    ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn test_state() -> AppState {
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(
                    ["crypto".to_string(), "stock".to_string()]
                        .into_iter()
                        .collect::<HashSet<_>>(),
                ),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

fn test_state_with_registry(toml: &str, skills: &[&str]) -> AppState {
    let path = std::env::temp_dir().join(format!(
        "loop_reply_registry_{}_{}.toml",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_file(path);
    let mut state = test_state();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn verify_summary(
    mode: crate::verifier::VerifyMode,
) -> crate::task_journal::TaskJournalVerifySummary {
    crate::task_journal::TaskJournalVerifySummary {
        mode,
        approved: true,
        needs_confirmation: true,
        ..Default::default()
    }
}

fn finalizer_summary(
    disposition: crate::finalize::FinalizerDisposition,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(disposition),
        ..Default::default()
    }
}

fn message_has_machine_key(text: &str, expected: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .and_then(|payload| {
            payload
                .pointer("/message_key")
                .and_then(serde_json::Value::as_str)
                .map(|value| value == expected)
        })
        .unwrap_or_else(|| text.contains(&format!("message_key={expected}")))
}

fn message_has_reason_code(text: &str, expected: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .and_then(|payload| {
            payload
                .pointer("/reason_code")
                .and_then(serde_json::Value::as_str)
                .map(|value| value == expected)
        })
        .unwrap_or_else(|| text.contains(&format!("reason_code={expected}")))
}

fn assert_missing_file_delivery_text(text: &str) {
    let trimmed = text.trim();
    assert!(!trimmed.is_empty());
    assert!(
        serde_json::from_str::<serde_json::Value>(trimmed).is_err(),
        "missing-file reply must not expose raw machine JSON: {text}"
    );
    assert!(
        !message_has_machine_key(text, "clawd.msg.delivery.file_not_found_path_next_step")
            && !message_has_machine_key(text, "clawd.msg.delivery.file_not_found_next_step"),
        "missing-file reply must not expose a message_key as user text: {text}"
    );
    assert!(
        !message_has_reason_code(text, "missing_file_delivery_not_found"),
        "missing-file reply must not expose a reason_code as user text: {text}"
    );
}

fn scalar_route_result() -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "extract scalar".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: Default::default(),
            semantic_kind: Default::default(),
            locator_hint: "package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn free_route_result() -> RouteResult {
    let mut route = scalar_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route
}

fn push_raw_plan_text(loop_state: &mut crate::agent_engine::LoopState, raw_plan_text: &str) {
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: Vec::new(),
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: raw_plan_text.to_string(),
            }),
            verify_result: None,
        });
}

fn plan_result_with_steps(steps: Vec<crate::PlanStep>) -> crate::PlanResult {
    crate::PlanResult {
        goal: "test goal".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps,
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    }
}

fn plan_result_with_raw_text(raw_plan_text: &str) -> crate::PlanResult {
    crate::PlanResult {
        raw_plan_text: raw_plan_text.to_string(),
        ..plan_result_with_steps(Vec::new())
    }
}

#[tokio::test]
async fn finalize_loop_reply_attaches_requested_control_machine_envelope() {
    let state = test_state();
    let task = claimed_task("task-control-envelope");
    let mut route = scalar_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.output_contract.semantic_kind = OutputSemanticKind::DocumentHeading;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_machine_fields": [
                    "decision_envelope.control_intent",
                    "decision_envelope.capability_ref"
                ]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.delivery_messages.push("# RustClaw".to_string());
    loop_state.output_vars.insert(
        "agent_loop.decision_envelope".to_string(),
        serde_json::json!({
            "control_intent": "act",
            "terminal_intent": "continue",
            "decision": "call_capability",
            "capability_ref": "fs_basic",
            "control_reason_code": "agent_loop_control_act_first_action"
        })
        .to_string(),
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return heading and decision envelope control fields",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should attach requested control envelope");

    let envelope = reply
        .messages
        .iter()
        .find_map(|message| {
            let payload = serde_json::from_str::<serde_json::Value>(message.trim()).ok()?;
            (payload
                .get("owner_layer")
                .and_then(serde_json::Value::as_str)
                == Some("agent_loop_control"))
            .then_some(payload)
        })
        .expect("agent_loop_control envelope");
    assert_eq!(
        envelope
            .pointer("/decision_envelope/control_intent")
            .and_then(serde_json::Value::as_str),
        Some("act")
    );
    assert_eq!(
        envelope
            .pointer("/decision_envelope/capability_ref")
            .and_then(serde_json::Value::as_str),
        Some("fs_basic")
    );
    assert!(envelope
        .pointer("/output_contract/contract_marker")
        .is_none());
    assert!(envelope
        .pointer("/output_contract/final_answer_shape")
        .and_then(serde_json::Value::as_str)
        .is_some());
    assert!(reply.text.contains("control_intent"));
    assert!(reply.text.contains("# RustClaw"));
}

#[tokio::test]
async fn finalize_loop_reply_does_not_attach_control_envelope_without_structured_request() {
    let state = test_state();
    let task = claimed_task("task-no-control-envelope");
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(scalar_route_result()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.delivery_messages.push("# RustClaw".to_string());
    loop_state.output_vars.insert(
        "agent_loop.first_act_decision_envelope".to_string(),
        serde_json::json!({
            "control_intent": "act",
            "terminal_intent": "continue",
            "decision": "call_capability",
            "capability_ref": "fs_basic"
        })
        .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.decision_envelope".to_string(),
        serde_json::json!({
            "control_intent": "answer",
            "terminal_intent": "answer",
            "decision": "synthesize_answer",
            "capability_ref": "synthesize_answer"
        })
        .to_string(),
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return heading",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should not attach control envelope");

    assert!(!reply.messages.iter().any(|message| {
        serde_json::from_str::<serde_json::Value>(message.trim())
            .ok()
            .and_then(|payload| {
                payload
                    .get("owner_layer")
                    .and_then(serde_json::Value::as_str)
                    .map(|owner| owner == "agent_loop_control")
            })
            .unwrap_or(false)
    }));
}

#[tokio::test]
async fn finalize_loop_reply_does_not_attach_control_envelope_from_route_machine_token() {
    let state = test_state();
    let task = claimed_task("task-control-envelope-route-token");
    let mut route = scalar_route_result();
    route.route_reason = "runtime requested control_intent=act machine token".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.delivery_messages.push("# RustClaw".to_string());
    loop_state.output_vars.insert(
        "agent_loop.first_act_decision_envelope".to_string(),
        serde_json::json!({
            "control_intent": "act",
            "terminal_intent": "continue",
            "decision": "call_capability",
            "capability_ref": "fs_basic"
        })
        .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.decision_envelope".to_string(),
        serde_json::json!({
            "control_intent": "answer",
            "terminal_intent": "answer",
            "decision": "synthesize_answer",
            "capability_ref": "synthesize_answer"
        })
        .to_string(),
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return heading",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should not attach control envelope from route machine token");

    assert!(!reply.messages.iter().any(|message| {
        serde_json::from_str::<serde_json::Value>(message.trim())
            .ok()
            .and_then(|payload| {
                payload
                    .get("owner_layer")
                    .and_then(serde_json::Value::as_str)
                    .map(|owner| owner == "agent_loop_control")
            })
            .unwrap_or(false)
    }));
}

fn ok_step_result(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    }
}

fn err_step_result(step_id: &str, skill: &str, error: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(error.to_string()),
        started_at: 1,
        finished_at: 2,
    }
}
