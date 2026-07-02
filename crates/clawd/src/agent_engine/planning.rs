use super::planning_actions::{
    build_plan_result, build_plan_result_with_notes, contains_unavailable_skill_action,
    has_executable_observation_or_action, has_tool_or_skill_observation, planned_action_skill_name,
};
use super::planning_followup::{
    has_authoritative_delivery, has_discussion_followup_action, is_delivery_failure_terminal_reply,
    is_discussion_followup_action, is_plain_respond_only_plan, last_executable_action,
    route_expects_terminal_user_answer, route_explicitly_requests_raw_command_output,
    should_preserve_terminal_followup_for_observed_finalize,
    terminal_reply_mentions_observed_missing_target,
};
#[cfg(test)]
use super::planning_parse::extract_xml_tool_call_steps;
use super::planning_parse::parse_single_plan_actions;
#[cfg(test)]
use super::planning_prompt::compact_skill_playbook_from_prompt;
use super::planning_prompt::{
    build_incremental_plan_prompt, build_lightweight_skill_playbooks_text,
    build_lightweight_skill_quick_index_text, build_lightweight_tool_spec,
    classify_planning_prompt_class, compact_lightweight_incremental_goal_context,
    contract_scoped_lightweight_planner_skill_scope, contract_scoped_planner_skill_scope,
    ensure_required_contract_block_present, incremental_prompt_spec_for_class,
    round1_prompt_spec_for_class, runtime_os_label, runtime_shell_label, PlanningPromptClass,
};
#[cfg(test)]
use super::planning_recent_artifacts::recent_artifacts_judgment_deterministic_plan_result;
#[cfg(test)]
use super::planning_registry_preference::registry_preferred_skill_matches_route;
#[cfg(test)]
use super::planning_registry_preference::registry_preferred_skill_names_for_route;
use super::planning_registry_preference::{
    actions_use_ad_hoc_command_without_route_preferred_skill,
    path_has_structured_document_extension,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::{
    attempt_ledger::build_attempt_ledger_compact,
    build_loop_history_compact, build_single_plan_prompt, build_skill_playbooks_text_scoped,
    build_skill_quick_index_text_scoped, build_turn_analysis_prompt_block,
    planning_route_markers::{
        route_allows_structured_candidate_read_target_repair,
        route_has_unresolved_clarify_or_locator_marker, route_reason_has_structural_marker,
    },
    AgentLoopGuardPolicy, LoopState, AGENT_TOOL_SPEC_PATH, PLAN_REPAIR_PROMPT_LOGICAL_PATH,
};
use crate::{llm_gateway, AgentAction, AppState, ClaimedTask, PlanKind, PlanResult, RouteResult};

#[path = "planning_scalar_count_filter.rs"]
mod scalar_count_filter;
#[cfg(test)]
use scalar_count_filter::scalar_count_filter_hint_for_route_or_turn;
use scalar_count_filter::{apply_scalar_count_filter_hint, scalar_count_filter_hint_from_route};

#[path = "action_route_locator_artifact.rs"]
mod action_route_locator_artifact;
#[cfg(test)]
#[path = "archive_database_aggregate_plan.rs"]
mod archive_database_aggregate_plan;
#[path = "concrete_respond_structural_observation.rs"]
mod concrete_respond_structural_observation;
#[path = "config_guard_capability_repair.rs"]
mod config_guard_capability_repair;
#[path = "configured_command_prefix.rs"]
mod configured_command_prefix;
#[path = "direct_observed_finalize_support.rs"]
mod direct_observed_finalize_support;
#[path = "directory_entry_group_locator.rs"]
mod directory_entry_group_locator;
#[path = "directory_unique_entry.rs"]
mod directory_unique_entry;
#[path = "dry_run_contract_plan.rs"]
mod dry_run_contract_plan;
#[path = "explicit_observed_paths.rs"]
mod explicit_observed_paths;
#[cfg(test)]
#[path = "filesystem_mutation_plan.rs"]
mod filesystem_mutation_plan;
#[path = "inline_transform_contract.rs"]
mod inline_transform_contract;
#[cfg(test)]
#[path = "kb_chain_plan.rs"]
mod kb_chain_plan;
#[path = "legacy_file_config_capabilities.rs"]
mod legacy_file_config_capabilities;
#[path = "media_artifact_plan.rs"]
mod media_artifact_plan;
#[path = "preferred_structured_action.rs"]
mod preferred_structured_action;
#[path = "read_range_action.rs"]
mod read_range_action;
#[path = "runtime_status_scalar_plan.rs"]
mod runtime_status_scalar_plan;
#[path = "runtime_surface_plan.rs"]
mod runtime_surface_plan;
#[path = "scalar_compare_observation.rs"]
mod scalar_compare_observation;
#[path = "scalar_count_deterministic_plan.rs"]
mod scalar_count_deterministic_plan;
#[path = "scalar_count_explicit_path.rs"]
mod scalar_count_explicit_path;
#[path = "session_alias_target_coverage.rs"]
mod session_alias_target_coverage;
#[path = "shell_sequence_part.rs"]
mod shell_sequence_part;
#[path = "single_target_structured_field_rewrite.rs"]
mod single_target_structured_field_rewrite;
#[path = "sqlite_table_listing_rewrite.rs"]
mod sqlite_table_listing_rewrite;
#[path = "structured_multi_field_read_rewrite.rs"]
mod structured_multi_field_read_rewrite;
#[path = "system_basic_action_path.rs"]
mod system_basic_action_path;
#[path = "value_string_list.rs"]
mod value_string_list;
use action_route_locator_artifact::*;
#[cfg(test)]
use archive_database_aggregate_plan::*;
use concrete_respond_structural_observation::*;
use config_guard_capability_repair::*;
use configured_command_prefix::*;
pub(in crate::agent_engine) use configured_command_prefix::{
    explicit_command_segment, explicit_execution_command_segment,
};
use direct_observed_finalize_support::*;
use directory_entry_group_locator::executed_step_is_successful_text_read;
#[cfg(test)]
use directory_entry_group_locator::*;
use directory_unique_entry::*;
use dry_run_contract_plan::*;
use explicit_observed_paths::*;
#[cfg(test)]
use filesystem_mutation_plan::*;
use inline_transform_contract::*;
#[cfg(test)]
use kb_chain_plan::*;
use legacy_file_config_capabilities::*;
use media_artifact_plan::*;
use preferred_structured_action::*;
use read_range_action::*;
use runtime_status_scalar_plan::*;
use runtime_surface_plan::*;
use scalar_compare_observation::*;
use scalar_count_deterministic_plan::*;
use scalar_count_explicit_path::*;
use session_alias_target_coverage::*;
use shell_sequence_part::*;
use single_target_structured_field_rewrite::*;
use sqlite_table_listing_rewrite::*;
use structured_multi_field_read_rewrite::*;
use system_basic_action_path::*;
use value_string_list::*;

pub(super) async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    turn_analysis_for_prompt: Option<&crate::intent_router::TurnAnalysis>,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Result<PlanResult, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let agent_runtime_identity = state.agent_runtime_identity_label().to_string();
    let planning_class = classify_planning_prompt_class(route_result, user_text, loop_state);
    let original_user_text_for_policy = crate::language_policy::task_original_user_text(task)
        .unwrap_or_else(|| user_text.to_string());
    let explicit_command_request = explicit_command_request_present(
        &state.policy.command_intent,
        &original_user_text_for_policy,
        route_result,
    );
    let explicit_command_scalar_path_current_workspace =
        explicit_command_scalar_path_current_workspace_should_prefer_run_cmd(
            &state.policy.command_intent,
            &original_user_text_for_policy,
            route_result,
        );
    let allow_structural_deterministic_plans = !explicit_command_request
        || (structural_contract_deterministic_plan_overrides_literal_command_guard(route_result)
            && !explicit_command_scalar_path_current_workspace);
    macro_rules! return_deterministic_plan {
        ($maybe_plan:expr, $reason_code:literal) => {
            if let Some(plan_result) = $maybe_plan {
                info!(
                    concat!($reason_code, " task_id={} round={}"),
                    task.task_id, loop_state.round_no
                );
                return Ok(plan_result_with_fallback_reason(plan_result, $reason_code));
            }
        };
    }

    return_deterministic_plan!(
        structured_dry_run_response_deterministic_plan_result(goal, route_result, loop_state),
        "plan_deterministic_structured_dry_run_response"
    );
    return_deterministic_plan!(
        inline_json_transform_deterministic_plan_result(
            goal,
            state,
            loop_state,
            &original_user_text_for_policy,
            route_result,
        ),
        "plan_deterministic_inline_json_transform"
    );
    if explicit_command_request
        && !allow_structural_deterministic_plans
        && runtime_status_query_kind(turn_analysis_for_prompt).is_none()
    {
        return_deterministic_plan!(
            explicit_command_deterministic_plan_result(
                state,
                goal,
                route_result,
                loop_state,
                &original_user_text_for_policy,
                turn_analysis_for_prompt,
            ),
            "plan_deterministic_explicit_command_run_cmd"
        );
    }
    return_deterministic_plan!(
        active_task_append_current_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            turn_analysis_for_prompt,
            auto_locator_path,
        ),
        "plan_deterministic_active_task_append_current_locator"
    );
    return_deterministic_plan!(
        runtime_status_scalar_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            turn_analysis_for_prompt,
        ),
        "plan_deterministic_runtime_status_scalar"
    );
    return_deterministic_plan!(
        runtime_status_scalar_info_fallback_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            turn_analysis_for_prompt,
        ),
        "plan_deterministic_runtime_status_scalar_info_fallback"
    );
    return_deterministic_plan!(
        http_download_artifact_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ),
        "plan_deterministic_http_download_artifact"
    );
    return_deterministic_plan!(
        hook_permission_surface_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ),
        "plan_deterministic_hook_permission_surface"
    );
    return_deterministic_plan!(
        clawcli_resume_surface_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ),
        "plan_deterministic_clawcli_resume_surface"
    );
    return_deterministic_plan!(
        subagent_bounded_batch_surface_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ),
        "plan_deterministic_subagent_bounded_batch_surface"
    );
    return_deterministic_plan!(
        subagent_review_boundary_surface_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ),
        "plan_deterministic_subagent_review_boundary_surface"
    );
    return_deterministic_plan!(
        async_job_start_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ),
        "plan_deterministic_async_job_start"
    );
    let recent_assistant_replies = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        crate::memory::build_recent_assistant_replies_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            3,
            220,
        )
    } else {
        "<omitted: lightweight_execution>".to_string()
    };
    let contract_skill_scope = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        contract_scoped_planner_skill_scope(route_result)
    } else {
        contract_scoped_lightweight_planner_skill_scope(route_result)
    };
    let skill_playbooks = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_playbooks_text_scoped(state, task, contract_skill_scope.as_ref())
    } else {
        build_lightweight_skill_playbooks_text(state, task, contract_skill_scope.as_ref())
    };
    let skill_quick_index = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_quick_index_text_scoped(state, task, contract_skill_scope.as_ref())
    } else {
        build_lightweight_skill_quick_index_text(state, task, contract_skill_scope.as_ref())
    };
    let tool_spec_template = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        crate::bootstrap::load_required_prompt_template_for_state(state, AGENT_TOOL_SPEC_PATH)
            .map_err(|e| e.to_string())?
            .0
    } else {
        build_lightweight_tool_spec(route_result, auto_locator_path)
    };
    let turn_analysis = build_turn_analysis_prompt_block(turn_analysis_for_prompt, route_result);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let attempt_ledger = build_attempt_ledger_compact(loop_state);
    let (prompt_name, prompt_source, prompt_version, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_name, prompt_logical_path) = round1_prompt_spec_for_class(planning_class);
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        (prompt_name, resolved.source, resolved.version, {
            let mut prompt = build_single_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                &agent_runtime_identity,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            );
            if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
                prompt.push_str(
                        "\n\n## Skill Quick Index (first-round routing hint)\nGoal: reduce misclassification while minimizing avoidable extra rounds.\n- Do NOT end round-1 with a generic chat-style final answer when a skill might be relevant.\n- In round-1, prioritize intent classification + missing-slot check, but finish immediately when one bounded resolution/current-runtime step can already complete the request safely.\n- Ask one concise clarification only when safe completion is truly blocked after current-turn text, immediate context, and bounded resolution/default inference have been used.\n- Use immediate `call_skill` in round-1 whenever intent is clear or can be completed by one bounded resolution/current-runtime step.\n",
                    );
                prompt.push_str(&skill_quick_index);
                prompt.push('\n');
            }
            prompt
        })
    } else {
        let history_compact = build_loop_history_compact(loop_state);
        // Phase 3.3 / observation history regression fix:
        // 之前这里只读 delivery_messages.last()。delivery_messages 仅承载最终 respond/交付
        // 文本，observation-only 步骤（fs_search/list_dir/read_file/run_cmd 等）的输出从不
        // 写入这里。结果是 round N+1 的 loop planner 看到 "Last round output: (none)"，
        // 完全看不到 round N 的工具输出，于是会重复同一观察步骤，最终触发 plan_unactionable
        // 兜底（i18n 模板被误用作 "provider unavailable" 文案）。
        // 真正记录每步输出的字段是 LoopState.last_output（agent_engine.rs 中
        // register_step_output / register_failed_step_output 都会维护）。优先使用它，
        // 仅在确无 step output 时回退到 delivery_messages，最后退化到占位符。
        let last_output = loop_state
            .last_output
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| crate::truncate_for_log(s))
            .or_else(|| loop_state.delivery_messages.last().cloned())
            .unwrap_or_else(|| "(none)".to_string());
        let (prompt_name, prompt_logical_path) = incremental_prompt_spec_for_class(planning_class);
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        let effective_goal = if matches!(planning_class, PlanningPromptClass::LightweightExecution)
        {
            compact_lightweight_incremental_goal_context(goal)
        } else {
            goal.to_string()
        };
        (
            prompt_name,
            resolved.source,
            resolved.version,
            build_incremental_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                &effective_goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                &agent_runtime_identity,
                loop_state.round_no,
                &history_compact,
                &attempt_ledger,
                &last_output,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            ),
        )
    };
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        prompt_name,
        &prompt_source,
        prompt_version.as_deref(),
        Some(loop_state.round_no),
    );
    info!(
        "{} loop_round_plan task_id={} round={} max_rounds={} max_steps={} multi_round_enabled={}",
        crate::highlight_tag("loop"),
        task.task_id,
        loop_state.round_no,
        policy.max_rounds,
        policy.max_steps,
        policy.multi_round_enabled
    );
    info!(
        "plan_llm_request task_id={} round={} planning_class={} prompt_chars={} tool_spec_chars={} playbooks_chars={} recent_replies_chars={} user_request={}",
        task.task_id,
        loop_state.round_no,
        planning_class.as_str(),
        prompt_text.chars().count(),
        tool_spec_template.chars().count(),
        skill_playbooks.chars().count(),
        recent_assistant_replies.chars().count(),
        crate::truncate_for_log(user_text)
    );
    ensure_required_contract_block_present(route_result, &prompt_text)?;
    let plan_raw = llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt_text,
        &prompt_source,
    )
    .await?;
    info!(
        "plan_llm_response task_id={} round={} raw={}",
        task.task_id,
        loop_state.round_no,
        crate::truncate_for_log(&plan_raw)
    );
    let parsed_actions = parse_single_plan_actions(&plan_raw, state, task).await;
    let mut initial_fallback_reason_code: Option<&'static str> = None;
    let initial_actions = parsed_actions
        .or_else(|| {
            let fallback = route_clarify_terminal_respond_fallback_actions(route_result);
            if fallback.is_some() {
                initial_fallback_reason_code =
                    Some("plan_parse_failed_route_clarify_terminal_respond");
                warn!(
                    "plan_parse_failed_using_route_clarify_terminal_respond task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback = plain_text_terminal_respond_fallback_actions(route_result, &plan_raw);
            if fallback.is_some() {
                initial_fallback_reason_code =
                    Some("plan_parse_failed_plain_text_terminal_respond");
                warn!(
                    "plan_parse_failed_using_plain_text_terminal_respond task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback = scalar_path_directory_locator_search_observation_plan(
                route_result,
                auto_locator_path,
                &original_user_text_for_policy,
            );
            if fallback.is_some() {
                initial_fallback_reason_code =
                    Some("plan_parse_failed_scalar_path_directory_locator_search");
                warn!(
                    "plan_parse_failed_using_scalar_path_directory_locator_search_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                scalar_content_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                initial_fallback_reason_code =
                    Some("plan_parse_failed_scalar_content_auto_locator");
                warn!(
                    "plan_parse_failed_using_scalar_content_auto_locator_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                scalar_path_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                initial_fallback_reason_code = Some("plan_parse_failed_scalar_path_auto_locator");
                warn!(
                    "plan_parse_failed_using_auto_locator_observation_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                file_facts_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                initial_fallback_reason_code = Some("plan_parse_failed_file_facts_auto_locator");
                warn!(
                    "plan_parse_failed_using_file_facts_auto_locator_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let fallback =
                generic_directory_auto_locator_observation_plan(route_result, auto_locator_path);
            if fallback.is_some() {
                initial_fallback_reason_code =
                    Some("plan_parse_failed_generic_directory_auto_locator");
                warn!(
                    "plan_parse_failed_using_generic_directory_auto_locator_plan task_id={} round={}",
                    task.task_id, loop_state.round_no
                );
            }
            fallback
        })
        .or_else(|| {
            let route = route_result?;
            if loop_state.has_tool_or_skill_output
                || !route_needs_workspace_respond_only_default_evidence(route)
            {
                return None;
            }
            warn!(
                "plan_parse_failed_using_workspace_default_evidence_plan task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            initial_fallback_reason_code = Some("plan_parse_failed_workspace_default_evidence");
            Some(workspace_summary_default_evidence_actions())
        })
        .map(|actions| {
            normalize_planned_actions_with_original_and_context(
                state,
                route_result,
                loop_state,
                user_text,
                Some(&original_user_text_for_policy),
                Some(goal),
                auto_locator_path,
                actions,
            )
        });
    let needs_repair = match initial_actions.as_ref() {
        Some(actions) => {
            should_force_actionable_plan_repair(state, route_result, loop_state, actions)
        }
        None => true,
    };
    let (plan_actions, plan_kind, raw_plan_text, planner_notes) = if needs_repair {
        let repair_reason =
            plan_repair_reason(state, route_result, loop_state, initial_actions.as_deref());
        warn!(
            "plan_repair_required task_id={} round={} reason={}",
            task.task_id, loop_state.round_no, repair_reason
        );
        // Planner-first: do not synthesize semantic repair plans from local keyword rules.
        // Repair either comes from the LLM repair prompt or, if safe, from the original
        // executable plan that the model already produced.
        match repair_plan_actions(
            state,
            task,
            goal,
            &turn_analysis,
            user_text,
            repair_reason,
            &tool_spec_template,
            &skill_playbooks,
            &attempt_ledger,
            &plan_raw,
            loop_state.round_no,
        )
        .await
        {
            Ok(repaired) => {
                let repaired_actions =
                    parse_single_plan_actions(&repaired, state, task)
                        .await
                        .map(|actions| {
                            normalize_planned_actions_with_original_and_context(
                                state,
                                route_result,
                                loop_state,
                                user_text,
                                Some(&original_user_text_for_policy),
                                Some(goal),
                                auto_locator_path,
                                actions,
                            )
                        });
                match repaired_actions {
                    Some(actions)
                        if !should_force_actionable_plan_repair(
                            state,
                            route_result,
                            loop_state,
                            &actions,
                        ) =>
                    {
                        (
                            actions,
                            PlanKind::Repair,
                            repaired,
                            planner_notes_for_repair_success(repair_reason, None),
                        )
                    }
                    Some(actions) => {
                        let second_repair_reason =
                            plan_repair_reason(state, route_result, loop_state, Some(&actions));
                        warn!(
                            "plan_repair_still_invalid task_id={} round={} reason={}",
                            task.task_id, loop_state.round_no, second_repair_reason
                        );
                        let second_repaired = repair_plan_actions(
                            state,
                            task,
                            goal,
                            &turn_analysis,
                            user_text,
                            second_repair_reason,
                            &tool_spec_template,
                            &skill_playbooks,
                            &attempt_ledger,
                            &repaired,
                            loop_state.round_no,
                        )
                        .await?;
                        let second_repaired_actions =
                            parse_single_plan_actions(&second_repaired, state, task)
                                .await
                                .map(|actions| {
                                    normalize_planned_actions_with_original_and_context(
                                        state,
                                        route_result,
                                        loop_state,
                                        user_text,
                                        Some(&original_user_text_for_policy),
                                        Some(goal),
                                        auto_locator_path,
                                        actions,
                                    )
                                });
                        match second_repaired_actions {
                            Some(second_actions)
                                if !should_force_actionable_plan_repair(
                                    state,
                                    route_result,
                                    loop_state,
                                    &second_actions,
                                ) =>
                            {
                                (
                                    second_actions,
                                    PlanKind::Repair,
                                    second_repaired,
                                    planner_notes_for_repair_success(
                                        repair_reason,
                                        Some(second_repair_reason),
                                    ),
                                )
                            }
                            Some(_) => {
                                let fallback_actions = initial_actions.as_ref().filter(|actions| {
                                    can_fallback_to_initial_plan_after_repair_failure(
                                        state,
                                        route_result,
                                        loop_state,
                                        actions,
                                    )
                                });
                                if let Some(actions) = fallback_actions {
                                    warn!(
                                        "plan_second_repair_invalid_fallback_to_initial task_id={} round={}",
                                        task.task_id, loop_state.round_no
                                    );
                                    (
                                        actions.clone(),
                                        if loop_state.round_no <= 1 {
                                            PlanKind::Single
                                        } else {
                                            PlanKind::Incremental
                                        },
                                        plan_raw.clone(),
                                        planner_notes_for_repair_fallback(
                                            "plan_second_repair_invalid_fallback_to_initial",
                                            initial_fallback_reason_code,
                                        ),
                                    )
                                } else {
                                    return Err(
                                        "repair plan still non-actionable after second repair"
                                            .to_string(),
                                    );
                                }
                            }
                            None => {
                                let fallback_actions = initial_actions.as_ref().filter(|actions| {
                                    can_fallback_to_initial_plan_after_repair_failure(
                                        state,
                                        route_result,
                                        loop_state,
                                        actions,
                                    )
                                });
                                if let Some(actions) = fallback_actions {
                                    warn!(
                                        "plan_second_repair_parse_failed_fallback_to_initial task_id={} round={}",
                                        task.task_id, loop_state.round_no
                                    );
                                    (
                                        actions.clone(),
                                        if loop_state.round_no <= 1 {
                                            PlanKind::Single
                                        } else {
                                            PlanKind::Incremental
                                        },
                                        plan_raw.clone(),
                                        planner_notes_for_repair_fallback(
                                            "plan_second_repair_parse_failed_fallback_to_initial",
                                            initial_fallback_reason_code,
                                        ),
                                    )
                                } else {
                                    return Err(
                                        "second repair plan parser failed: no executable steps"
                                            .to_string(),
                                    );
                                }
                            }
                        }
                    }
                    None => {
                        let fallback_actions = initial_actions.as_ref().filter(|actions| {
                            can_fallback_to_initial_plan_after_repair_failure(
                                state,
                                route_result,
                                loop_state,
                                actions,
                            )
                        });
                        if let Some(actions) = fallback_actions {
                            warn!(
                                "plan_repair_parse_failed_fallback_to_initial task_id={} round={}",
                                task.task_id, loop_state.round_no
                            );
                            (
                                actions.clone(),
                                if loop_state.round_no <= 1 {
                                    PlanKind::Single
                                } else {
                                    PlanKind::Incremental
                                },
                                plan_raw.clone(),
                                planner_notes_for_repair_fallback(
                                    "plan_repair_parse_failed_fallback_to_initial",
                                    initial_fallback_reason_code,
                                ),
                            )
                        } else {
                            return Err(
                                "single plan parser failed: no executable steps".to_string()
                            );
                        }
                    }
                }
            }
            Err(err) => {
                let fallback_actions = initial_actions.as_ref().filter(|actions| {
                    can_fallback_to_initial_plan_after_repair_failure(
                        state,
                        route_result,
                        loop_state,
                        actions,
                    )
                });
                if let Some(actions) = fallback_actions {
                    warn!(
                        "plan_repair_llm_failed_fallback_to_initial task_id={} round={} error={}",
                        task.task_id,
                        loop_state.round_no,
                        crate::truncate_for_log(&err)
                    );
                    (
                        actions.clone(),
                        if loop_state.round_no <= 1 {
                            PlanKind::Single
                        } else {
                            PlanKind::Incremental
                        },
                        plan_raw.clone(),
                        planner_notes_for_repair_fallback(
                            "plan_repair_llm_failed_fallback_to_initial",
                            initial_fallback_reason_code,
                        ),
                    )
                } else {
                    return Err(err);
                }
            }
        }
    } else {
        (
            initial_actions.expect("checked Some above"),
            if loop_state.round_no <= 1 {
                PlanKind::Single
            } else {
                PlanKind::Incremental
            },
            plan_raw.clone(),
            planner_notes_for_initial_fallback(initial_fallback_reason_code),
        )
    };
    let plan_result = build_plan_result_with_notes(
        goal,
        &raw_plan_text,
        plan_kind,
        &plan_actions,
        &planner_notes,
    );
    let labels = plan_result.step_labels();
    info!(
        "act_split_trace task_id={} round={} split_steps={}",
        task.task_id,
        loop_state.round_no,
        serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(plan_result)
}

fn explicit_command_scalar_path_current_workspace_should_prefer_run_cmd(
    command_runtime: &crate::CommandIntentRuntime,
    original_user_text: &str,
    route_result: Option<&RouteResult>,
) -> bool {
    explicit_command_request_present(command_runtime, original_user_text, route_result)
        && route_result.is_some_and(|route| {
            let scalar_path_contract =
                route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly);
            let route_preserves_explicit_command = route_reason_has_structural_marker(
                route,
                "explicit_command_preserves_structured_observation_contract",
            );
            let current_workspace_path =
                route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace;
            let auto_locator_path_conflict = route.output_contract.locator_kind
                == crate::OutputLocatorKind::Path
                && !route.output_contract.locator_hint.trim().is_empty()
                && route_preserves_explicit_command;
            scalar_path_contract
                && (current_workspace_path || auto_locator_path_conflict)
                && route_preserves_explicit_command
        })
}

fn plan_result_with_fallback_reason(
    mut plan_result: PlanResult,
    reason_code: &'static str,
) -> PlanResult {
    let note = format!("fallback_reason_code={reason_code}");
    let notes = plan_result.planner_notes.trim();
    if notes.is_empty() {
        plan_result.planner_notes = note;
    } else if !notes.split_whitespace().any(|item| item == note) {
        plan_result.planner_notes = format!("{notes} {note}");
    }
    plan_result
}

fn plain_text_terminal_respond_fallback_actions(
    route_result: Option<&RouteResult>,
    raw_plan_text: &str,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    let content = raw_plan_text.trim();
    if content.is_empty() || raw_plan_text_looks_like_structured_plan_fragment(content) {
        return None;
    }
    let chat_like_route = route.is_resume_discussion_mode()
        || route.uses_chat_finalizer()
        || route_reason_has_structural_marker(route, "pure_chat_agent_loop_submode");
    if !chat_like_route
        || route.needs_clarify
        || route.wants_file_delivery
        || route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::None
        || !route.output_contract_is_unclassified()
        || route.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route.output_contract.locator_hint.trim().is_empty()
        || !route_allows_model_language_terminal_respond(Some(route))
    {
        return None;
    }
    Some(vec![AgentAction::Respond {
        content: content.to_string(),
    }])
}

pub(super) fn route_clarify_terminal_respond_fallback_actions(
    route_result: Option<&RouteResult>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    let content = route.clarify_question.trim();
    if !route.needs_clarify
        || content.is_empty()
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || route.output_contract.requires_content_evidence
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::None
        || !route.output_contract_is_unclassified()
    {
        return None;
    }
    Some(vec![AgentAction::Respond {
        content: content.to_string(),
    }])
}

fn raw_plan_text_looks_like_structured_plan_fragment(content: &str) -> bool {
    let content = content.trim_start_matches('\u{feff}').trim_start();
    let lower = content.to_ascii_lowercase();
    lower.starts_with('{')
        || lower.starts_with('[')
        || lower.starts_with("<tool_call")
        || lower.starts_with("<tool>")
}

fn planner_notes_for_initial_fallback(reason_code: Option<&str>) -> String {
    reason_code
        .map(|reason| format!("fallback_reason_code={reason}"))
        .unwrap_or_default()
}

fn planner_notes_for_repair_success(first_reason: &str, second_reason: Option<&str>) -> String {
    let mut notes = vec![format!("repair_reason_code={first_reason}")];
    if let Some(second_reason) = second_reason {
        notes.push(format!("second_repair_reason_code={second_reason}"));
    }
    notes.join(" ")
}

fn planner_notes_for_repair_fallback(reason_code: &str, initial_reason: Option<&str>) -> String {
    let mut notes = vec![format!("fallback_reason_code={reason_code}")];
    if let Some(initial_reason) = initial_reason {
        notes.push(format!("initial_fallback_reason_code={initial_reason}"));
    }
    notes.join(" ")
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "planning_recent_artifacts_tests.rs"]
mod recent_artifacts_tests;

#[cfg(test)]
#[path = "planning_scalar_count_tests.rs"]
mod scalar_count_tests;
