use super::planning_actions::{
    build_plan_result_with_notes, contains_unavailable_skill_action,
    has_executable_observation_or_action, has_tool_or_skill_observation, planned_action_skill_name,
};
use super::planning_followup::{
    has_authoritative_delivery, has_discussion_followup_action, is_delivery_failure_terminal_reply,
    is_discussion_followup_action, is_plain_respond_only_plan, last_executable_action,
    loop_state_has_pre_loop_locator_clarify_candidate, route_expects_terminal_user_answer,
    route_explicitly_requests_raw_command_output,
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
use super::planning_registry_preference::registry_preferred_skill_matches_route;
#[cfg(test)]
use super::planning_registry_preference::registry_preferred_skill_names_for_route;
use super::planning_registry_preference::{
    actions_use_ad_hoc_command_without_route_preferred_skill,
    path_has_structured_document_extension,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
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

/// Planner-visible tool and skill inventory for one loop round.
///
/// This helper only prepares prompt/tool-library material. It must not build a
/// `PlanResult`, choose a capability, or short-circuit the planner LLM.
struct PlannerToolLibrary<'a> {
    state: &'a AppState,
    task: &'a ClaimedTask,
    planning_class: PlanningPromptClass,
    route_result: Option<&'a RouteResult>,
    auto_locator_path: Option<&'a str>,
    skill_scope: Option<BTreeSet<String>>,
}

impl<'a> PlannerToolLibrary<'a> {
    fn new(
        state: &'a AppState,
        task: &'a ClaimedTask,
        planning_class: PlanningPromptClass,
        route_result: Option<&'a RouteResult>,
        auto_locator_path: Option<&'a str>,
    ) -> Self {
        let skill_scope = if matches!(planning_class, PlanningPromptClass::OpenPlanning) {
            contract_scoped_planner_skill_scope(route_result)
        } else {
            contract_scoped_lightweight_planner_skill_scope(route_result)
        };
        Self {
            state,
            task,
            planning_class,
            route_result,
            auto_locator_path,
            skill_scope,
        }
    }

    fn is_open_planning(&self) -> bool {
        matches!(self.planning_class, PlanningPromptClass::OpenPlanning)
    }

    fn uses_compact_tool_library(&self) -> bool {
        !self.is_open_planning() || self.skill_scope.is_some()
    }

    fn skill_playbooks(&self) -> String {
        if self.uses_compact_tool_library() {
            build_lightweight_skill_playbooks_text(self.state, self.task, self.skill_scope.as_ref())
        } else {
            build_skill_playbooks_text_scoped(self.state, self.task, self.skill_scope.as_ref())
        }
    }

    fn skill_quick_index(&self) -> String {
        if self.uses_compact_tool_library() {
            build_lightweight_skill_quick_index_text(
                self.state,
                self.task,
                self.skill_scope.as_ref(),
            )
        } else {
            build_skill_quick_index_text_scoped(self.state, self.task, self.skill_scope.as_ref())
        }
    }

    fn tool_spec(&self) -> Result<String, String> {
        if self.uses_compact_tool_library() {
            Ok(build_lightweight_tool_spec(
                self.route_result,
                self.auto_locator_path,
            ))
        } else {
            crate::bootstrap::load_required_prompt_template_for_state(
                self.state,
                AGENT_TOOL_SPEC_PATH,
            )
            .map(|resolved| {
                let capability_map =
                    crate::capability_map::build_capability_map_for_task(self.state, self.task);
                let mut spec = String::new();
                spec.push_str("runtime_capability_map_v1");
                spec.push('\n');
                spec.push_str(&capability_map);
                spec.push('\n');
                spec.push('\n');
                spec.push_str(&resolved.0);
                spec
            })
            .map_err(|err| err.to_string())
        }
    }
}

#[path = "planning_scalar_count_filter.rs"]
mod scalar_count_filter;
#[cfg(test)]
use scalar_count_filter::scalar_count_filter_hint_for_route_or_turn;
use scalar_count_filter::{apply_scalar_count_filter_hint, scalar_count_filter_hint_from_route};

#[path = "action_route_locator_artifact.rs"]
mod action_route_locator_artifact;
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
#[path = "explicit_observed_paths.rs"]
mod explicit_observed_paths;
#[path = "inline_transform_contract.rs"]
mod inline_transform_contract;
#[path = "legacy_file_config_capabilities.rs"]
mod legacy_file_config_capabilities;
#[path = "media_artifact_plan.rs"]
mod media_artifact_plan;
#[path = "planner_abort_recovery.rs"]
mod planner_abort_recovery;
#[path = "preferred_structured_action.rs"]
mod preferred_structured_action;
#[path = "read_range_action.rs"]
mod read_range_action;
#[path = "runtime_status_scalar_plan.rs"]
mod runtime_status_scalar_plan;
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
use concrete_respond_structural_observation::*;
use config_guard_capability_repair::*;
#[cfg(test)]
pub(in crate::agent_engine) use configured_command_prefix::explicit_command_segment;
pub(in crate::agent_engine) use configured_command_prefix::explicit_machine_syntax_command_segment;
use configured_command_prefix::*;
use direct_observed_finalize_support::*;
use directory_entry_group_locator::executed_step_is_successful_text_read;
#[cfg(test)]
use directory_entry_group_locator::*;
use directory_unique_entry::*;
use explicit_observed_paths::*;
use inline_transform_contract::*;
use legacy_file_config_capabilities::*;
use media_artifact_plan::*;
use planner_abort_recovery::*;
use preferred_structured_action::*;
use read_range_action::*;
use runtime_status_scalar_plan::*;
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
    boundary_envelope_for_prompt: Option<&crate::intent_router::BoundaryEnvelope>,
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
    let planner_tool_library =
        PlannerToolLibrary::new(state, task, planning_class, route_result, auto_locator_path);
    let skill_playbooks = planner_tool_library.skill_playbooks();
    let skill_quick_index = planner_tool_library.skill_quick_index();
    let tool_spec_template = planner_tool_library.tool_spec()?;
    let turn_analysis = build_turn_analysis_prompt_block(
        turn_analysis_for_prompt,
        boundary_envelope_for_prompt,
        route_result,
    );
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
    let initial_actions = parse_single_plan_actions(&plan_raw, state, task)
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
                                        ),
                                    )
                                } else {
                                    if let Some((actions, raw)) = try_compact_abort_recovery_plan(
                                        state,
                                        task,
                                        goal,
                                        &turn_analysis,
                                        user_text,
                                        route_result,
                                        loop_state,
                                        auto_locator_path,
                                        Some(&original_user_text_for_policy),
                                        &tool_spec_template,
                                        &skill_playbooks,
                                        &attempt_ledger,
                                        &plan_raw,
                                        Some(&second_repaired),
                                    )
                                    .await?
                                    {
                                        (
                                            actions,
                                            PlanKind::Repair,
                                            raw,
                                            planner_notes_for_repair_success(
                                                repair_reason,
                                                Some("planner_abort_compact_retry"),
                                            ),
                                        )
                                    } else {
                                        return Err(
                                            "plan_second_repair_parse_failed_no_executable_steps"
                                                .to_string(),
                                        );
                                    }
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
                                ),
                            )
                        } else {
                            if let Some((actions, raw)) = try_compact_abort_recovery_plan(
                                state,
                                task,
                                goal,
                                &turn_analysis,
                                user_text,
                                route_result,
                                loop_state,
                                auto_locator_path,
                                Some(&original_user_text_for_policy),
                                &tool_spec_template,
                                &skill_playbooks,
                                &attempt_ledger,
                                &plan_raw,
                                Some(&repaired),
                            )
                            .await?
                            {
                                (
                                    actions,
                                    PlanKind::Repair,
                                    raw,
                                    planner_notes_for_repair_success(
                                        repair_reason,
                                        Some("planner_abort_compact_retry"),
                                    ),
                                )
                            } else {
                                return Err("plan_parse_failed_no_executable_steps".to_string());
                            }
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
            String::new(),
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

#[allow(clippy::too_many_arguments)]
async fn try_compact_abort_recovery_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    turn_analysis: &str,
    user_text: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    original_user_text_for_policy: Option<&str>,
    tool_spec_template: &str,
    skill_playbooks: &str,
    attempt_ledger: &str,
    first_raw_plan: &str,
    latest_raw_plan: Option<&str>,
) -> Result<Option<(Vec<AgentAction>, String)>, String> {
    let Some((actions, raw)) = compact_retry_plan_actions(
        state,
        task,
        PlannerAbortRecoveryInput {
            goal,
            turn_analysis,
            user_text,
            tool_spec: tool_spec_template,
            skill_playbooks,
            attempt_ledger,
            first_raw_plan,
            latest_raw_plan,
            round_no: loop_state.round_no,
            route_result,
            loop_state,
        },
    )
    .await?
    else {
        return Ok(None);
    };
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text_for_policy,
        Some(goal),
        auto_locator_path,
        actions,
    );
    if should_force_actionable_plan_repair(state, route_result, loop_state, &actions) {
        warn!(
            "planner_abort_compact_retry_non_actionable task_id={} round={}",
            task.task_id, loop_state.round_no
        );
        return Ok(None);
    }
    Ok(Some((actions, raw)))
}

#[cfg(test)]
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

#[cfg(test)]
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

fn planner_notes_for_repair_success(first_reason: &str, second_reason: Option<&str>) -> String {
    let mut notes = vec![format!("repair_reason_code={first_reason}")];
    if let Some(second_reason) = second_reason {
        notes.push(format!("second_repair_reason_code={second_reason}"));
    }
    notes.join(" ")
}

fn planner_notes_for_repair_fallback(reason_code: &str) -> String {
    format!("fallback_reason_code={reason_code}")
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
