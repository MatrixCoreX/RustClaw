use super::planning_actions::{
    build_plan_result, contains_unavailable_skill_action, has_executable_observation_or_action,
    has_tool_or_skill_observation, planned_action_skill_name,
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
    classify_planning_prompt_class, contract_scoped_planner_skill_scope,
    ensure_required_contract_block_present, round1_prompt_spec_for_class, runtime_os_label,
    runtime_shell_label, PlanningPromptClass,
};
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
    planning_numeric_limits::first_ascii_integer_limit,
    planning_route_markers::{
        route_allows_structured_candidate_read_target_repair, route_reason_has_structural_marker,
    },
    AgentLoopGuardPolicy, LoopState, AGENT_TOOL_SPEC_PATH,
    LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH, PLAN_REPAIR_PROMPT_LOGICAL_PATH,
};
use crate::{llm_gateway, AgentAction, AppState, ClaimedTask, PlanKind, PlanResult, RouteResult};

#[path = "planning_scalar_count_filter.rs"]
mod scalar_count_filter;
use scalar_count_filter::{
    apply_scalar_count_filter_hint, scalar_count_filter_hint_for_route_or_turn,
    scalar_count_filter_hint_from_route,
};

#[path = "planning_split_01_action_supports_direct_observed_finalize.rs"]
mod planning_split_01_action_supports_direct_observed_finalize;
#[path = "planning_split_02_session_alias_targets_missing_from.rs"]
mod planning_split_02_session_alias_targets_missing_from;
#[path = "planning_split_03_directory_entry_groups_auto_locator.rs"]
mod planning_split_03_directory_entry_groups_auto_locator;
#[path = "planning_split_04_scalar_count_filter_deterministic_plan.rs"]
mod planning_split_04_scalar_count_filter_deterministic_plan;
#[path = "planning_split_05_preferred_structured_action_for_contract.rs"]
mod planning_split_05_preferred_structured_action_for_contract;
#[path = "planning_split_06_directory_has_unique_entry_for.rs"]
mod planning_split_06_directory_has_unique_entry_for;
#[path = "planning_split_07_route_has_inline_transform_contract.rs"]
mod planning_split_07_route_has_inline_transform_contract;
#[path = "planning_split_08_scalar_count_explicit_count_path.rs"]
mod planning_split_08_scalar_count_explicit_count_path;
#[path = "planning_split_09_strip_configured_command_prefix.rs"]
mod planning_split_09_strip_configured_command_prefix;
#[path = "planning_split_10_canonicalize_legacy_file_config_capabilities.rs"]
mod planning_split_10_canonicalize_legacy_file_config_capabilities;
#[path = "planning_split_11_is_read_range_action.rs"]
mod planning_split_11_is_read_range_action;
#[path = "planning_split_12_rewrite_structured_multi_field_read.rs"]
mod planning_split_12_rewrite_structured_multi_field_read;
#[path = "planning_split_13_string_list_from_value.rs"]
mod planning_split_13_string_list_from_value;
#[path = "planning_split_14_runtime_status_scalar_deterministic_plan.rs"]
mod planning_split_14_runtime_status_scalar_deterministic_plan;
#[path = "planning_split_15_system_basic_action_path_and.rs"]
mod planning_split_15_system_basic_action_path_and;
#[path = "planning_split_16_action_targets_route_locator_artifact.rs"]
mod planning_split_16_action_targets_route_locator_artifact;
#[path = "planning_split_17_executed_step_scalar_compare_observation.rs"]
mod planning_split_17_executed_step_scalar_compare_observation;
#[path = "planning_split_18_action_observed_paths_for_explicit.rs"]
mod planning_split_18_action_observed_paths_for_explicit;
#[path = "planning_split_19_rewrite_single_target_structured_field.rs"]
mod planning_split_19_rewrite_single_target_structured_field;
#[path = "planning_split_20_rewrite_sqlite_table_listing_plan.rs"]
mod planning_split_20_rewrite_sqlite_table_listing_plan;
#[path = "planning_split_21_shell_sequence_part_can_run.rs"]
mod planning_split_21_shell_sequence_part_can_run;
#[path = "planning_split_22_concrete_respond_has_structural_observation.rs"]
mod planning_split_22_concrete_respond_has_structural_observation;
use planning_split_01_action_supports_direct_observed_finalize::*;
use planning_split_02_session_alias_targets_missing_from::*;
use planning_split_03_directory_entry_groups_auto_locator::*;
use planning_split_04_scalar_count_filter_deterministic_plan::*;
use planning_split_05_preferred_structured_action_for_contract::*;
use planning_split_06_directory_has_unique_entry_for::*;
use planning_split_07_route_has_inline_transform_contract::*;
use planning_split_08_scalar_count_explicit_count_path::*;
use planning_split_09_strip_configured_command_prefix::*;
pub(in crate::agent_engine) use planning_split_09_strip_configured_command_prefix::{
    explicit_command_segment, explicit_execution_command_segment,
};
use planning_split_10_canonicalize_legacy_file_config_capabilities::*;
use planning_split_11_is_read_range_action::*;
use planning_split_12_rewrite_structured_multi_field_read::*;
use planning_split_13_string_list_from_value::*;
use planning_split_14_runtime_status_scalar_deterministic_plan::*;
use planning_split_15_system_basic_action_path_and::*;
use planning_split_16_action_targets_route_locator_artifact::*;
use planning_split_17_executed_step_scalar_compare_observation::*;
use planning_split_18_action_observed_paths_for_explicit::*;
use planning_split_19_rewrite_single_target_structured_field::*;
use planning_split_20_rewrite_sqlite_table_listing_plan::*;
use planning_split_21_shell_sequence_part_can_run::*;
use planning_split_22_concrete_respond_has_structural_observation::*;

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
    let planning_class = classify_planning_prompt_class(route_result, user_text, loop_state);
    let original_user_text_for_policy = crate::language_policy::task_original_user_text(task)
        .unwrap_or_else(|| user_text.to_string());
    let explicit_command_request = explicit_command_request_present(
        &state.policy.command_intent,
        &original_user_text_for_policy,
        route_result,
    );
    let allow_structural_deterministic_plans = !explicit_command_request
        || structural_contract_deterministic_plan_overrides_literal_command_guard(route_result);
    if let Some(plan_result) = inline_json_transform_deterministic_plan_result(
        goal,
        state,
        loop_state,
        &original_user_text_for_policy,
        route_result,
    ) {
        info!(
            "plan_deterministic_inline_json_transform task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if explicit_command_request {
        if let Some(plan_result) = explicit_command_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
            turn_analysis_for_prompt,
        ) {
            info!(
                "plan_deterministic_explicit_command_run_cmd task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
    }
    if let Some(plan_result) = active_task_append_current_locator_deterministic_plan_result(
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_active_task_append_current_locator task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = scalar_count_filter_deterministic_plan_result(
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_scalar_count_filter task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = contract_hint_preferred_action_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        &original_user_text_for_policy,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_contract_hint_preferred_action task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = package_manager_detect_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        auto_locator_path,
    ) {
        info!(
            "plan_deterministic_package_manager_detect task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = package_manager_dry_run_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        &original_user_text_for_policy,
    ) {
        info!(
            "plan_deterministic_package_manager_dry_run task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = runtime_status_scalar_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
    ) {
        info!(
            "plan_deterministic_runtime_status_scalar task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = runtime_status_scalar_info_fallback_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        turn_analysis_for_prompt,
    ) {
        info!(
            "plan_deterministic_runtime_status_scalar_info_fallback task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if let Some(plan_result) = service_status_deterministic_plan_result(
        state,
        goal,
        route_result,
        loop_state,
        &original_user_text_for_policy,
    ) {
        info!(
            "plan_deterministic_service_status task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(plan_result);
    }
    if allow_structural_deterministic_plans {
        if let Some(plan_result) = directory_purpose_representative_reads_after_find_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_purpose_representative_reads task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = structured_keys_deterministic_plan_result(
            state,
            goal,
            &original_user_text_for_policy,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_structured_keys task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = content_presence_quoted_literal_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_content_presence_quoted_literal task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = git_repository_state_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_git_repository_state task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = recent_scalar_file_pair_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            &original_user_text_for_policy,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_recent_scalar_file_pair task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = recent_scalar_current_workspace_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
        ) {
            info!(
                "plan_deterministic_recent_scalar_current_workspace task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = scalar_path_directory_locator_search_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_scalar_path_directory_locator_search task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = scalar_content_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_scalar_content_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = scalar_path_auto_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_scalar_path_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = quantity_compare_pair_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            Some(&original_user_text_for_policy),
        ) {
            info!(
                "plan_deterministic_quantity_compare_pair_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = file_facts_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_file_facts_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = existence_with_path_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_existence_with_path_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = file_paths_locator_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_file_paths_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = file_names_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_file_names_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_compare_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
        ) {
            info!(
                "plan_deterministic_directory_compare_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_entry_groups_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_entry_groups_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_purpose_extension_inventory_deterministic_plan_result(
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_purpose_extension_inventory task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_purpose_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_purpose_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = generic_path_content_log_analyze_deterministic_plan_result(
            goal,
            state,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_generic_path_content_log_analyze task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = directory_tree_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            user_text,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_directory_tree_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = archive_read_deterministic_plan_result(
            goal,
            state,
            route_result,
            loop_state,
            auto_locator_path,
            &original_user_text_for_policy,
        ) {
            info!(
                "plan_deterministic_archive_read task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = archive_pack_deterministic_plan_result(
            goal,
            state,
            route_result,
            loop_state,
            &original_user_text_for_policy,
            Some(&original_user_text_for_policy),
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_archive_pack task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) =
            archive_unpack_deterministic_plan_result(goal, state, route_result, loop_state)
        {
            info!(
                "plan_deterministic_archive_unpack task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = archive_list_auto_locator_deterministic_plan_result(
            goal,
            state,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_archive_list_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
        if let Some(plan_result) = content_excerpt_summary_auto_locator_deterministic_plan_result(
            state,
            goal,
            route_result,
            loop_state,
            auto_locator_path,
        ) {
            info!(
                "plan_deterministic_content_excerpt_summary_auto_locator task_id={} round={}",
                task.task_id, loop_state.round_no
            );
            return Ok(plan_result);
        }
    }
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
    let contract_skill_scope = contract_scoped_planner_skill_scope(route_result);
    let skill_playbooks = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_playbooks_text_scoped(state, task, contract_skill_scope.as_ref())
    } else {
        build_lightweight_skill_playbooks_text(state, task)
    };
    let skill_quick_index = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
        build_skill_quick_index_text_scoped(state, task, contract_skill_scope.as_ref())
    } else {
        build_lightweight_skill_quick_index_text(state, task)
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
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            LOOP_INCREMENTAL_PLAN_PROMPT_LOGICAL_PATH,
        )
        .map_err(|e| e.to_string())?;
        (
            "loop_incremental_plan_prompt",
            resolved.source,
            resolved.version,
            build_incremental_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
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
    let initial_actions = parsed_actions
        .or_else(|| {
            let fallback = scalar_path_directory_locator_search_observation_plan(
                route_result,
                auto_locator_path,
                &original_user_text_for_policy,
            );
            if fallback.is_some() {
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
    let (plan_actions, plan_kind, raw_plan_text) = if needs_repair {
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
                        (actions, PlanKind::Repair, repaired)
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
                                (second_actions, PlanKind::Repair, second_repaired)
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
        )
    };
    let plan_result = build_plan_result(goal, &raw_plan_text, plan_kind, &plan_actions);
    let labels = plan_result.step_labels();
    info!(
        "act_split_trace task_id={} round={} split_steps={}",
        task.task_id,
        loop_state.round_no,
        serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(plan_result)
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
