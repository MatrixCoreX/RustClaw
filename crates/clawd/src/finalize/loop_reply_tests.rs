use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    agent_context_allows_observed_output_language_fallback,
    append_compound_file_delivery_token_from_route,
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
    direct_non_builtin_skill_raw_answer, direct_path_from_active_bound_inventory,
    direct_publishable_observed_answer, direct_quantity_comparison_from_compare_paths,
    direct_raw_command_output_projection, direct_rustclaw_config_risk_answer,
    direct_scalar_observed_answer, direct_structured_observed_answer,
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
    observed_execution_without_publishable_delivery_outcome,
    observed_execution_without_publishable_delivery_reply, observed_synthesis_unavailable_reply,
    path_batch_size_comparison_answer, prefer_observed_answer_for_exact_contract,
    preferred_route_clarify_question,
    replace_delivery_with_deterministic_current_workspace_dirs_overview_answer,
    replace_delivery_with_deterministic_directory_purpose_answer,
    replace_delivery_with_deterministic_execution_failed_step_answer,
    replace_delivery_with_deterministic_observed_execution_status_answer,
    replace_delivery_with_deterministic_quantity_comparison_answer,
    replace_delivery_with_deterministic_recent_artifacts_judgment_answer,
    replace_delivery_with_deterministic_rustclaw_config_risk_answer,
    replace_delivery_with_latest_tail_read_range_answer,
    replace_delivery_with_observed_markdown_heading_scalar,
    replace_raw_observation_delivery_with_synthesis, resolve_file_token_from_auto_locator_answer,
    route_prefers_language_rendered_execution_failed_step, route_structured_clarify_context,
    should_attach_execution_summary, should_drop_passthrough_delivery_for_content_evidence,
    should_return_missing_file_delivery_reply, should_try_observed_output_language_fallback,
    successful_delivery_final_status, verify_summary_requires_resume_confirmation,
};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    AgentRuntimeConfig, AppState, ClaimedTask, IntentOutputContract, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

#[path = "loop_reply_execution_summary_tests.rs"]
mod execution_summary_tests;

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

#[path = "loop_reply_service_status_tests.rs"]
mod service_status_tests;

#[path = "loop_reply_error_finalize_tests.rs"]
mod error_finalize_tests;

#[path = "loop_reply_scalar_direct_tests.rs"]
mod scalar_direct_tests;

#[path = "loop_reply_file_delivery_tests.rs"]
mod file_delivery_tests;

#[path = "loop_reply_file_missing_tests.rs"]
mod file_missing_tests;

#[path = "loop_reply_delivery_backfill_tests.rs"]
mod delivery_backfill_tests;

#[path = "loop_reply_content_evidence_passthrough_tests.rs"]
mod content_evidence_passthrough_tests;

#[path = "loop_reply_git_state_tests.rs"]
mod git_state_tests;

#[path = "loop_reply_markdown_scalar_tests.rs"]
mod markdown_scalar_tests;

#[path = "loop_reply_matrix_shape_tests.rs"]
mod matrix_shape_tests;

#[path = "loop_reply_tail_read_tests.rs"]
mod tail_read_tests;

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
        ask_mode: crate::AskMode::planner_execute_plain(),
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
