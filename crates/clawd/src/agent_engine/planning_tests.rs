use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;

use super::{
    action_supports_structured_direct_observed_finalize, action_targets_config_edit,
    actions_use_ad_hoc_command_without_route_preferred_skill, apply_scalar_count_filter_hint,
    broaden_default_read_range_for_structured_text, build_lightweight_skill_playbooks_text,
    build_lightweight_skill_quick_index_text, build_lightweight_tool_spec,
    can_fallback_to_initial_plan_after_repair_failure, classify_planning_prompt_class,
    compact_lightweight_incremental_goal_context, compact_skill_playbook_from_prompt,
    contains_unavailable_skill_action, contract_scoped_lightweight_planner_skill_scope,
    contract_scoped_planner_skill_scope,
    directory_purpose_representative_read_actions_after_find_result,
    enforce_output_contract_tool_args, ensure_content_excerpt_summary_has_bounded_content,
    ensure_required_contract_block_present, ensure_workspace_synthesis_has_default_text_evidence,
    file_facts_auto_locator_observation_plan, fill_missing_read_range_path_from_route_locator,
    generic_directory_auto_locator_observation_plan, generic_path_content_log_analyze_target_path,
    has_pre_observation_structured_output_shape, incremental_prompt_spec_for_class,
    inject_structural_extension_filter_for_directory_inventory,
    inject_synthesize_answer_for_bare_placeholder_respond, is_bare_last_output_placeholder,
    normalize_action_schema_aliases, normalize_archive_basic_schema_aliases,
    normalize_fs_basic_schema_aliases, normalize_git_basic_schema_aliases,
    normalize_planned_actions, normalize_planned_actions_with_original,
    normalize_planned_actions_with_original_and_context, normalize_system_basic_schema_aliases,
    normalize_transform_schema_aliases, observation_only_plan_can_finalize_from_direct_output,
    plan_repair_reason, plan_result_with_fallback_reason, planner_notes_for_repair_fallback,
    planner_notes_for_repair_success, preferred_run_cmd_for_contract_hint,
    registry_preferred_skill_names_for_route, repair_guard_config_default_path_for_invalid_locator,
    replace_file_delivery_respond_only_with_path_observation,
    replace_scalar_count_plan_with_count_inventory,
    replace_scalar_path_respond_only_with_auto_locator_observation,
    replace_workspace_synthesis_respond_only_plan, resolve_directory_locator_for_dir_compare,
    rewrite_active_bound_target_observations_to_matching_locator_hint,
    rewrite_archive_basic_short_archive_to_active_bound_target,
    rewrite_archive_pack_plan_to_archive_basic, rewrite_archive_unpack_run_cmd_to_archive_basic,
    rewrite_config_change_preview_to_config_edit_plan,
    rewrite_config_mutation_plan_only_to_config_edit_plan,
    rewrite_config_mutation_to_config_edit_closed_loop,
    rewrite_config_validation_read_plan_to_validate,
    rewrite_docker_readonly_run_cmd_to_docker_basic, rewrite_extract_field_alias_args,
    rewrite_git_show_file_at_rev_capability_fs_reads,
    rewrite_observed_terminal_synthesis_concrete_respond,
    rewrite_pre_observation_concrete_respond_to_placeholder,
    rewrite_process_ps_run_cmd_to_process_basic, rewrite_readonly_git_run_cmd_to_git_basic,
    rewrite_readonly_runtime_status_run_cmd_to_system_basic,
    rewrite_rustclaw_config_risk_assessment_to_guard, rewrite_rustclaw_config_validation_to_guard,
    rewrite_service_status_plan_to_service_control,
    rewrite_session_alias_delivery_observations_to_route_locator,
    rewrite_sqlite_count_query_to_requested_schema_column,
    rewrite_sqlite_schema_version_plan_to_db_basic, rewrite_sqlite_table_listing_plan_to_db_basic,
    rewrite_sqlite_table_probe_to_requested_schema_value,
    rewrite_sqlite_user_version_plan_to_db_basic,
    rewrite_terminal_placeholder_respond_to_synthesize_answer,
    rewrite_terminal_synthesis_placeholder_respond,
    rewrite_unresolved_template_arg_multi_file_read_plan, round1_prompt_spec_for_class,
    route_contract_defers_literal_command_to_planner, route_uses_runtime_owned_observed_finalizer,
    scalar_content_auto_locator_observation_plan, scalar_count_filter_hint_for_route_or_turn,
    scalar_path_auto_locator_observation_plan,
    scalar_path_directory_locator_search_observation_plan, should_force_actionable_plan_repair,
    strip_directory_read_range_after_inventory_dir, strip_file_lines_count_before_tail_read_range,
    strip_intermediate_synthesize_before_later_execution,
    strip_terminal_discussion_for_direct_skill_passthrough,
    strip_terminal_discussion_for_observed_finalize,
    strip_terminal_discussion_for_scalar_path_observation,
    strip_terminal_placeholder_respond_for_exact_listing_contract,
    strip_unresolved_template_reads_after_inventory_dir, structured_field_selectors, LoopState,
    PlannerToolLibrary, PlanningPromptClass,
};
use crate::agent_engine::{
    CLAWD_CONTINUE_ON_ERROR_ARG, CLAWD_LITERAL_COMMAND_ARG, CLAWD_RUNTIME_ASYNC_JOB_START_ARG,
};
use crate::{
    AgentAction, AgentRuntimeConfig, AppState, AskMode, ClaimedTask, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, PlanKind,
    ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind, SkillViewsSnapshot, ToolsPolicy,
    DEFAULT_AGENT_ID,
};
use serde_json::{json, Value};

#[test]
fn planner_notes_record_repair_reason_codes() {
    assert_eq!(
        planner_notes_for_repair_success("plan_missing_terminal_user_answer", None),
        "repair_reason_code=plan_missing_terminal_user_answer"
    );
    assert_eq!(
        planner_notes_for_repair_success(
            "plan_missing_terminal_user_answer",
            Some("content_evidence_requires_content_observation")
        ),
        "repair_reason_code=plan_missing_terminal_user_answer second_repair_reason_code=content_evidence_requires_content_observation"
    );
}

#[test]
fn planner_notes_record_repair_fallback_reason_codes() {
    assert_eq!(
        planner_notes_for_repair_fallback("plan_repair_llm_failed_fallback_to_initial"),
        "fallback_reason_code=plan_repair_llm_failed_fallback_to_initial"
    );
}

#[test]
fn deterministic_plan_reason_code_appends_machine_note() {
    let plan = crate::PlanResult {
        goal: "g".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: Vec::new(),
        planner_notes: "repair_reason_code=existing".to_string(),
        plan_kind: PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let annotated =
        plan_result_with_fallback_reason(plan, "plan_deterministic_scalar_path_auto_locator");

    assert_eq!(
        annotated.planner_notes,
        "repair_reason_code=existing fallback_reason_code=plan_deterministic_scalar_path_auto_locator"
    );
}

fn planned_call<'a>(action: &'a AgentAction) -> Option<(&'a str, &'a Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        _ => None,
    }
}

fn planned_call_is(action: &AgentAction, name: &str, action_name: &str) -> bool {
    planned_call(action).is_some_and(|(tool, args)| {
        tool == name && args.get("action").and_then(Value::as_str) == Some(action_name)
    })
}

fn expect_planned_call<'a>(action: &'a AgentAction, name: &str, action_name: &str) -> &'a Value {
    let Some((tool, args)) = planned_call(action) else {
        panic!("expected {name}.{action_name} call, got {action:?}");
    };
    assert_eq!(tool, name);
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some(action_name)
    );
    args
}

fn assert_planner_supplied_skill_call_preserved(
    state: &AppState,
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    user_text: Option<&str>,
    context_text: Option<&str>,
    skill: &str,
    action_name: &str,
    args: Value,
) -> Value {
    let action = AgentAction::CallSkill {
        skill: skill.to_string(),
        args,
    };
    let AgentAction::CallSkill {
        skill: policy_skill,
        args: policy_args,
    } = &action
    else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(
            Some(route),
            policy_skill,
            policy_args
        )
        .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        state,
        Some(route),
        loop_state,
        goal,
        user_text,
        context_text,
        None,
        vec![action],
    );
    normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, skill, action_name)
                .then(|| expect_planned_call(action, skill, action_name).clone())
        })
        .unwrap_or_else(|| {
            panic!(
                "planner-supplied {skill}.{action_name} action should be preserved: {normalized:?}"
            )
        })
}

fn assert_planner_supplied_tool_call_preserved(
    state: &AppState,
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    user_text: Option<&str>,
    context_text: Option<&str>,
    tool: &str,
    action_name: &str,
    args: Value,
) -> Value {
    let action = AgentAction::CallTool {
        tool: tool.to_string(),
        args,
    };
    let AgentAction::CallTool {
        tool: policy_tool,
        args: policy_args,
    } = &action
    else {
        unreachable!("test action is a tool call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(
            Some(route),
            policy_tool,
            policy_args
        )
        .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        state,
        Some(route),
        loop_state,
        goal,
        user_text,
        context_text,
        None,
        vec![action],
    );
    normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, tool, action_name)
                .then(|| expect_planned_call(action, tool, action_name).clone())
        })
        .unwrap_or_else(|| {
            panic!(
                "planner-supplied {tool}.{action_name} action should be preserved: {normalized:?}"
            )
        })
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
            "clawd_planning_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
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
                skills_list: Arc::new(HashSet::new()),
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

fn test_state_with_enabled_skills(skills: &[&str]) -> AppState {
    let state = test_state();
    let enabled: HashSet<String> = skills.iter().map(|skill| (*skill).to_string()).collect();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: None,
        skills_list: Arc::new(enabled),
    });
    state
}

fn action_capability_and_action<'a>(
    action: &'a AgentAction,
    capability: &str,
    action_name: &str,
) -> Option<&'a Value> {
    match action {
        AgentAction::CallSkill { skill, args } if skill == capability => Some(args),
        AgentAction::CallTool { tool, args } if tool == capability => Some(args),
        _ => None,
    }
    .filter(|args| args.get("action").and_then(Value::as_str) == Some(action_name))
}

fn test_state_with_registry() -> AppState {
    let state = test_state();
    let registry_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(HashSet::new()),
    });
    state
}

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "test-task".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn base_route_result() -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    }
}

#[test]
fn backend_identity_metadata_respond_rewrites_to_runtime_identity() {
    let mut state = test_state();
    state.core.llm_providers = vec![Arc::new(crate::LlmProviderRuntime {
        config: claw_core::config::LlmProviderConfig {
            name: "vendor-mimo".to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "http://fixture.invalid".to_string(),
            api_key: "fixture".to_string(),
            model: "mimo-v2.5-pro".to_string(),
            context_window_tokens: None,
            priority: 1,
            timeout_seconds: 5,
            max_concurrency: 1,
            params: claw_core::config::LlmProviderParams::default(),
        },
        client: reqwest::Client::new(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })];
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.route_reason =
        "agent_display_name_hint_backend_metadata_removed; pure_chat_agent_loop_submode"
            .to_string();
    let actions = vec![AgentAction::Respond {
        content: "你好，我是 MiMo-v2.5-pro，由小米 MiMo 团队开发。".to_string(),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "Briefly tell me 你是谁，回答用中文",
        Some("Briefly tell me 你是谁，回答用中文"),
        Some("Briefly tell me 你是谁，回答用中文"),
        None,
        actions,
    );

    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "RustClaw"
    ));

    let mut route_without_marker = base_route_result();
    route_without_marker.route_reason = "pure_chat_agent_loop_submode".to_string();
    let actions = vec![AgentAction::Respond {
        content: "MiMo-v2.5-pro".to_string(),
    }];
    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route_without_marker),
        &loop_state,
        "Which backend model is selected?",
        Some("Which backend model is selected?"),
        Some("Which backend model is selected?"),
        None,
        actions,
    );
    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "MiMo-v2.5-pro"
    ));
}

fn should_force_plan_repair(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    should_force_actionable_plan_repair(&test_state(), route_result, loop_state, actions)
}

#[test]
fn pre_loop_locator_candidate_plain_respond_does_not_force_plan_repair() {
    let mut route = base_route_result();
    route.route_reason =
        "resolved_directory_observation_clarify_repair; executable_contract_preserved_for_agent_loop"
            .to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.requires_content_evidence = false;
    let mut loop_state = LoopState::new(1);
    loop_state.output_vars.insert(
        "pre_loop_clarify_candidates".to_string(),
        json!(["background_only_locator"]).to_string(),
    );
    let actions = vec![AgentAction::Respond {
        content: "Please provide the target file path.".to_string(),
    }];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}

fn repair_reason(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Option<&[AgentAction]>,
) -> &'static str {
    plan_repair_reason(&test_state(), route_result, loop_state, actions)
}

fn loop_state_with_required_session_alias_targets(targets: &[&str]) -> LoopState {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "required_session_alias_targets".to_string(),
        serde_json::to_string(&targets).unwrap(),
    );
    loop_state
}

fn route_result(
    ask_mode: AskMode,
    requires_content_evidence: bool,
    response_shape: OutputResponseShape,
) -> RouteResult {
    RouteResult {
        ask_mode,
        resolved_intent: "test".to_string(),
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
            response_shape,
            requires_content_evidence,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: Default::default(),
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn delivery_route_result() -> RouteResult {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        false,
        OutputResponseShape::FileToken,
    );
    route.output_contract.delivery_required = true;
    route
}

fn actions_as_json(actions: &[AgentAction]) -> serde_json::Value {
    serde_json::to_value(actions).expect("serialize")
}

// Test bodies live in functionally named submodules to keep this helper file maintainable.
#[path = "planning_tests/archive_and_inline_transform.rs"]
mod archive_and_inline_transform;
#[path = "planning_tests/archive_list_capability.rs"]
mod archive_list_capability;
#[path = "planning_tests/archive_pack_unpack_capability.rs"]
mod archive_pack_unpack_capability;
#[path = "planning_tests/bounded_log_slice_registry.rs"]
mod bounded_log_slice_registry;
#[path = "planning_tests/capability_read_sqlite_existence.rs"]
mod capability_read_sqlite_existence;
#[path = "planning_tests/config_guard_capability_repair.rs"]
mod config_guard_capability_repair;
#[path = "planning_tests/config_structured_field_reads.rs"]
mod config_structured_field_reads;
#[path = "planning_tests/content_excerpt_and_log_synthesis.rs"]
mod content_excerpt_and_log_synthesis;
#[path = "planning_tests/content_excerpt_log_slice_boundaries.rs"]
mod content_excerpt_log_slice_boundaries;
#[path = "planning_tests/contract_hint_agent_loop.rs"]
mod contract_hint_agent_loop;
#[path = "planning_tests/contract_hints_and_selectors.rs"]
mod contract_hints_and_selectors;
#[path = "planning_tests/delivery_archive_config_edit.rs"]
mod delivery_archive_config_edit;
#[path = "planning_tests/directory_listing_capability_scope.rs"]
mod directory_listing_capability_scope;
#[path = "planning_tests/directory_locator_and_workspace_summary.rs"]
mod directory_locator_and_workspace_summary;
#[path = "planning_tests/dry_run_contracts.rs"]
mod dry_run_contracts;
#[path = "planning_tests/existence_summary_metadata.rs"]
mod existence_summary_metadata;
#[path = "planning_tests/explicit_command_sequences.rs"]
mod explicit_command_sequences;
#[path = "planning_tests/file_paths_workspace_and_raw_command.rs"]
mod file_paths_workspace_and_raw_command;
#[path = "planning_tests/git_runtime_status_and_observation.rs"]
mod git_runtime_status_and_observation;
#[path = "planning_tests/kb_chain.rs"]
mod kb_chain;
#[path = "planning_tests/log_analyze_with_summary_policy.rs"]
mod log_analyze_with_summary_policy;
#[path = "planning_tests/log_excerpt_quantity_and_skill_policy.rs"]
mod log_excerpt_quantity_and_skill_policy;
#[path = "planning_tests/missing_paths_and_multi_target_metadata.rs"]
mod missing_paths_and_multi_target_metadata;
#[path = "planning_tests/nl_failure_regressions.rs"]
mod nl_failure_regressions;
#[path = "planning_tests/observed_finalize_followup.rs"]
mod observed_finalize_followup;
#[path = "planning_tests/runtime_surface_plans.rs"]
mod runtime_surface_plans;
#[path = "planning_tests/scalar_count_and_hidden_entries.rs"]
mod scalar_count_and_hidden_entries;
#[path = "planning_tests/scalar_path_and_inventory_repair.rs"]
mod scalar_path_and_inventory_repair;
#[path = "planning_tests/schema_and_template_aliases.rs"]
mod schema_and_template_aliases;
#[path = "planning_tests/service_status_capability_routes.rs"]
mod service_status_capability_routes;
#[path = "planning_tests/session_alias_and_content_evidence.rs"]
mod session_alias_and_content_evidence;
#[path = "planning_tests/structured_keys_and_scalar_fields.rs"]
mod structured_keys_and_scalar_fields;
#[path = "planning_tests/system_basic_aliases_and_quantity.rs"]
mod system_basic_aliases_and_quantity;
#[path = "planning_tests/task_execution_async_lifecycle.rs"]
mod task_execution_async_lifecycle;
#[path = "planning_tests/terminal_placeholder_rewrite.rs"]
mod terminal_placeholder_rewrite;
#[path = "planning_tests/terminal_synthesis_and_directory_inventory.rs"]
mod terminal_synthesis_and_directory_inventory;
