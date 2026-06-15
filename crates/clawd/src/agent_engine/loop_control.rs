use serde_json::{json, Value};
use tracing::{info, warn};

use super::support::LoopBudgetProfile;
use super::{
    append_progress_hint, attempt_ledger, encode_progress_i18n, ensure_task_running,
    execute_actions_once, load_agent_loop_guard_policy, prepare_round_actions, push_round_trace,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, AskReply, ClaimedTask, RouteResult};

#[path = "loop_control_answer_recovery.rs"]
mod loop_control_answer_recovery;
#[path = "loop_control_answer_recovery_parse.rs"]
mod loop_control_answer_recovery_parse;
#[path = "loop_control_answer_recovery_text.rs"]
mod loop_control_answer_recovery_text;
#[path = "loop_control_local_health_recovery.rs"]
mod loop_control_local_health_recovery;

use loop_control_answer_recovery::*;
use loop_control_answer_recovery_parse::*;
use loop_control_answer_recovery_text::*;
use loop_control_local_health_recovery::*;

fn has_authoritative_delivery(loop_state: &LoopState) -> bool {
    !loop_state.delivery_messages.is_empty()
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
        || loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
}

fn reply_final_status_is_clarify(reply: &AskReply) -> bool {
    reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.final_status)
        .is_some_and(|status| {
            matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
        })
}

fn route_expects_terminal_user_answer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required {
        return false;
    }
    !matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn route_requires_direct_candidate_for_observed_stop(route_result: &RouteResult) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        && crate::contract_matrix::final_answer_shape_for_output_contract(
            &route_result.output_contract,
        )
        .is_some_and(|shape| shape.allows_model_language())
}

fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::Think { .. } => false,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. } => false,
    })
}

fn has_executable_observation_or_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::SynthesizeAnswer { .. }
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredRespondTerminalIntent {
    terminal_intent: String,
    clarify_reason_code: Option<String>,
    missing_slot: Option<String>,
    message_key: Option<String>,
    field_path: Option<String>,
    locator_kind: Option<String>,
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn raw_plan_steps(raw_plan_text: &str) -> Vec<Value> {
    let Some(value) =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_plan_text)
    else {
        return Vec::new();
    };
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        return steps.clone();
    }
    if let Some(actions) = value.get("actions").and_then(Value::as_array) {
        return actions.clone();
    }
    value.as_array().cloned().unwrap_or_default()
}

fn structured_respond_terminal_intent_from_object(
    value: &Value,
) -> Option<StructuredRespondTerminalIntent> {
    let terminal_intent = string_field(value, &["terminal_intent"])?.to_ascii_lowercase();
    if !matches!(
        terminal_intent.as_str(),
        "answer" | "clarify" | "cannot_proceed" | "needs_confirmation" | "continue"
    ) {
        return None;
    }
    Some(StructuredRespondTerminalIntent {
        terminal_intent,
        clarify_reason_code: string_field(value, &["clarify_reason_code"]).map(str::to_string),
        missing_slot: string_field(value, &["missing_slot"]).map(str::to_string),
        message_key: string_field(value, &["message_key"]).map(str::to_string),
        field_path: string_field(value, &["field_path"]).map(str::to_string),
        locator_kind: string_field(value, &["locator_kind"]).map(str::to_string),
    })
}

fn structured_respond_terminal_intent_from_plan_step(
    step: &crate::PlanStep,
) -> Option<StructuredRespondTerminalIntent> {
    (step.action_type == "respond")
        .then(|| structured_respond_terminal_intent_from_object(&step.args))?
}

fn structured_respond_terminal_intent_from_raw_step(
    step: &Value,
) -> Option<StructuredRespondTerminalIntent> {
    let raw_type = string_field(step, &["type", "action_type", "action"])?.to_ascii_lowercase();
    (raw_type == "respond").then(|| structured_respond_terminal_intent_from_object(step))?
}

fn structured_respond_terminal_intent_from_plan(
    plan: &crate::PlanResult,
) -> Option<StructuredRespondTerminalIntent> {
    plan.steps
        .iter()
        .find_map(structured_respond_terminal_intent_from_plan_step)
        .or_else(|| {
            raw_plan_steps(&plan.raw_plan_text)
                .iter()
                .find_map(structured_respond_terminal_intent_from_raw_step)
        })
}

fn actions_allow_structured_respond_terminal_intent(actions: &[AgentAction]) -> bool {
    actions.iter().all(|action| {
        matches!(
            action,
            AgentAction::Respond { .. } | AgentAction::Think { .. }
        )
    })
}

fn apply_structured_respond_clarify_to_loop_state(
    loop_state: &mut LoopState,
    intent: &StructuredRespondTerminalIntent,
) -> RoundOutcome {
    loop_state.pending_user_input_required = true;
    loop_state.output_vars.insert(
        "agent_loop.terminal_intent".to_string(),
        "clarify".to_string(),
    );
    for (key, value) in [
        (
            "agent_loop.clarify_reason_code",
            intent.clarify_reason_code.as_deref(),
        ),
        ("agent_loop.missing_slot", intent.missing_slot.as_deref()),
        ("agent_loop.message_key", intent.message_key.as_deref()),
        ("agent_loop.field_path", intent.field_path.as_deref()),
        ("agent_loop.locator_kind", intent.locator_kind.as_deref()),
    ] {
        if let Some(value) = value {
            loop_state
                .output_vars
                .insert(key.to_string(), value.to_string());
        }
    }
    loop_state.history_compact.push(format!(
        "round={} structured_respond_terminal_intent=clarify missing_slot={}",
        loop_state.round_no,
        intent.missing_slot.as_deref().unwrap_or("")
    ));
    RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("structured_respond_clarify".to_string()),
        next_goal_hint: None,
        no_progress: false,
    }
}

fn last_executable_action(actions: &[AgentAction]) -> Option<&AgentAction> {
    actions.iter().rev().find(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn action_reads_text_content(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { .. } => return false,
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    let normalized_skill = skill.trim().replace('-', "_").to_ascii_lowercase();
    if matches!(normalized_skill.as_str(), "read_file" | "doc_parse") {
        return true;
    }
    normalized_skill == "system_basic"
        && args
            .get("action")
            .and_then(|value| value.as_str())
            .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
            .unwrap_or(false)
}

fn route_needs_workspace_text_evidence_before_observed_finalize(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape == crate::OutputResponseShape::Free
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.locator_hint.trim().is_empty()
}

fn structured_scalar_equality_observation_can_finalize(
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::RecentScalarEqualityCheck
        && !route_result.output_contract.delivery_required
        && has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && super::observed_output::structured_scalar_equality_direct_answer(
            None,
            route_result,
            loop_state,
            None,
        )
        .is_some()
}

fn latest_path_batch_facts_all_missing(loop_state: &LoopState) -> bool {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || step.skill != "system_basic" {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
            continue;
        }
        let Some(facts) = value.get("facts").and_then(|value| value.as_array()) else {
            return false;
        };
        if facts.is_empty() {
            return false;
        }
        return facts
            .iter()
            .all(|fact| fact.get("exists").and_then(|value| value.as_bool()) == Some(false));
    }
    false
}

pub(crate) fn requested_success_marker(
    _agent_run_context: Option<&AgentRunContext>,
) -> Option<&'static str> {
    None
}

fn observed_answer_contains_required_success_marker(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    marker: &str,
) -> bool {
    super::observed_output::extract_direct_answer_from_generic_output(loop_state, agent_run_context)
        .is_some_and(|answer| answer.contains(marker))
        || super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some_and(|answer| answer.contains(marker))
}

fn should_stop_for_observed_finalize(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if loop_state.execution_recipe.is_active()
        && !matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
    {
        return false;
    }
    if loop_state.execution_recipe.needs_validation() {
        return false;
    }
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify
        || !loop_state.has_tool_or_skill_output
        || has_authoritative_delivery(loop_state)
    {
        return false;
    }
    if route_needs_workspace_text_evidence_before_observed_finalize(route_result)
        && !has_discussion_followup_action(actions)
        && !last_executable_action(actions).is_some_and(action_reads_text_content)
    {
        return false;
    }
    let required_success_marker = requested_success_marker(agent_run_context);
    let has_direct_observed_answer =
        super::observed_output::extract_direct_answer_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some();
    if structured_scalar_equality_observation_can_finalize(route_result, loop_state, actions) {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && has_direct_observed_answer
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if quantity_comparison_one_sentence_needs_model_language_before_stop(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if super::observed_output::route_disallows_direct_observation_passthrough(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        && loop_state.round_no < loop_state.max_rounds
        && latest_path_batch_facts_all_missing(loop_state)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if has_direct_observed_answer
        && route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        if super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return required_success_marker.is_none_or(|marker| {
                observed_answer_contains_required_success_marker(
                    agent_run_context,
                    loop_state,
                    marker,
                )
            });
        }
        if super::observed_output::scalar_route_prefers_structured_observed_answer(
            route_result,
            loop_state,
        ) && super::observed_output::extract_direct_answer_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return required_success_marker.is_none_or(|marker| {
                observed_answer_contains_required_success_marker(
                    agent_run_context,
                    loop_state,
                    marker,
                )
            });
        }
    }
    let can_stop = has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && if route_requires_direct_candidate_for_observed_stop(route_result) {
            has_direct_observed_answer
        } else {
            super::observed_output::has_observed_answer_candidates(loop_state)
        };
    can_stop
        && required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        })
}

fn quantity_comparison_one_sentence_needs_model_language_before_stop(
    route_result: &RouteResult,
) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && route_result.output_contract.response_shape == crate::OutputResponseShape::OneSentence
        && crate::contract_matrix::final_answer_shape_for_output_contract(
            &route_result.output_contract,
        )
        .is_some_and(|shape| shape.allows_model_language())
}

fn evaluate_round_outcome(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    outcome: &RoundOutcome,
) -> bool {
    if outcome.had_error {
        info!(
            "loop_round_stop task_id={} round={} reason=had_error",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if let Some(reason) = &outcome.stop_signal {
        if reason == "recoverable_failure_continue_round" {
            if !policy.multi_round_enabled {
                info!(
                    "loop_round_stop task_id={} round={} reason=recoverable_failure_multi_round_disabled",
                    task.task_id, loop_state.round_no
                );
                return true;
            }
            if loop_state.round_no >= loop_state.max_rounds {
                if loop_state.recoverable_failure_extra_rounds_used
                    >= policy.recoverable_failure_extra_rounds
                {
                    info!(
                        "loop_round_stop task_id={} round={} reason=recoverable_failure_extra_rounds_exhausted used={} limit={}",
                        task.task_id,
                        loop_state.round_no,
                        loop_state.recoverable_failure_extra_rounds_used,
                        policy.recoverable_failure_extra_rounds
                    );
                    return true;
                }
                loop_state.recoverable_failure_extra_rounds_used += 1;
                loop_state.max_rounds += 1;
                info!(
                    "loop_round_extend task_id={} round={} reason={} new_max_rounds={} used_extra={}",
                    task.task_id,
                    loop_state.round_no,
                    reason,
                    loop_state.max_rounds,
                    loop_state.recoverable_failure_extra_rounds_used
                );
            }
            loop_state.consecutive_no_progress = 0;
            info!(
                "loop_round_continue task_id={} round={} reason={}",
                task.task_id, loop_state.round_no, reason
            );
            return false;
        }
        info!(
            "loop_round_stop task_id={} round={} reason={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            reason,
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return true;
    }
    if outcome.executed_actions == 0 {
        info!(
            "loop_round_stop task_id={} round={} reason=no_actions",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if outcome.no_progress {
        loop_state.consecutive_no_progress += 1;
    } else {
        loop_state.consecutive_no_progress = 0;
    }
    if loop_state.consecutive_no_progress > policy.no_progress_limit {
        info!(
            "loop_round_stop task_id={} round={} reason=no_progress limit={} count={}",
            task.task_id,
            loop_state.round_no,
            policy.no_progress_limit,
            loop_state.consecutive_no_progress
        );
        return true;
    }
    if !policy.multi_round_enabled {
        info!(
            "loop_round_stop task_id={} round={} reason=multi_round_disabled",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if loop_state.round_no >= loop_state.max_rounds {
        info!(
            "loop_round_stop task_id={} round={} reason=max_rounds reached={}",
            task.task_id, loop_state.round_no, loop_state.max_rounds
        );
        return true;
    }
    false
}

async fn run_agent_round(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<RoundOutcome, String> {
    info!(
        "loop_round_start task_id={} round={} max_rounds={} total_steps={} tool_calls_total={}",
        task.task_id,
        loop_state.round_no,
        loop_state.max_rounds,
        loop_state.total_steps_executed,
        loop_state.tool_calls_total
    );
    let prepared_round = prepare_round_actions(
        state,
        task,
        goal,
        user_text,
        policy,
        loop_state,
        agent_run_context,
    )
    .await?;
    push_round_trace(loop_state, goal, &prepared_round);
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, route_result);
    maybe_record_agent_decides_shadow_first_action_attribution(
        policy,
        task,
        agent_run_context,
        route_result,
        budget_profile,
        &prepared_round.actions,
        loop_state,
    );
    if let Some(intent) = structured_respond_terminal_intent_from_plan(&prepared_round.plan_result)
        .filter(|intent| intent.terminal_intent == "clarify")
        .filter(|_| actions_allow_structured_respond_terminal_intent(&prepared_round.actions))
    {
        let outcome = apply_structured_respond_clarify_to_loop_state(loop_state, &intent);
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return Ok(outcome);
    }
    let actions = prepared_round.actions;
    let mut outcome = execute_actions_once(
        state,
        task,
        goal,
        user_text,
        &actions,
        loop_state,
        policy,
        agent_run_context,
    )
    .await?;
    if outcome.stop_signal.is_none()
        && should_stop_for_observed_finalize(agent_run_context, loop_state, &actions)
    {
        outcome.stop_signal = Some("observed_output_ready".to_string());
    }
    info!(
        "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
        task.task_id,
        loop_state.round_no,
        outcome.executed_actions,
        outcome.no_progress,
        outcome.stop_signal.as_deref().unwrap_or(""),
        crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
    );
    Ok(outcome)
}

fn initial_execution_recipe_spec(
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> crate::execution_recipe::ExecutionRecipeSpec {
    if let Some(spec) = agent_run_context.and_then(|ctx| ctx.execution_recipe_hint) {
        return spec;
    }
    let _ = (goal, user_text);
    warn!(
        "execution_recipe_no_hint_bypass_local_detector route_available={} user_request_available={}",
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .is_some(),
        agent_run_context
            .and_then(|ctx| ctx.user_request.as_deref())
            .is_some_and(|text| !text.trim().is_empty())
    );
    crate::execution_recipe::ExecutionRecipeSpec::default()
}

fn maybe_record_agent_decides_shadow_attribution(
    policy: &AgentLoopGuardPolicy,
    task: &ClaimedTask,
    agent_run_context: Option<&AgentRunContext>,
    route_result: Option<&RouteResult>,
    budget_profile: LoopBudgetProfile,
    loop_state: &mut LoopState,
) {
    if !policy.records_agent_decides_attribution() {
        return;
    }
    let Some(route) = route_result else {
        return;
    };
    loop_state.rollout_attribution.push(
        crate::task_journal::TaskJournalRolloutAttribution::agent_decides_shadow_snapshot(
            route,
            budget_profile.as_str(),
            Some(boundary_context_snapshot_json(
                task,
                policy,
                agent_run_context,
                route_result,
                budget_profile,
            )),
        ),
    );
}

fn maybe_record_agent_decides_shadow_first_action_attribution(
    policy: &AgentLoopGuardPolicy,
    task: &ClaimedTask,
    agent_run_context: Option<&AgentRunContext>,
    route_result: Option<&RouteResult>,
    budget_profile: LoopBudgetProfile,
    actions: &[AgentAction],
    loop_state: &mut LoopState,
) {
    if !policy.records_agent_decides_attribution() || loop_state.round_no != 1 {
        return;
    }
    let Some(route) = route_result else {
        return;
    };
    if loop_state
        .rollout_attribution
        .iter()
        .any(|item| item.event == "agent_decides_shadow_first_action")
    {
        return;
    }
    loop_state.rollout_attribution.push(
        crate::task_journal::TaskJournalRolloutAttribution::agent_decides_shadow_first_action(
            route,
            budget_profile.as_str(),
            actions,
            Some(boundary_context_snapshot_json(
                task,
                policy,
                agent_run_context,
                route_result,
                budget_profile,
            )),
        ),
    );
}

pub(super) fn boundary_context_snapshot_json(
    task: &ClaimedTask,
    policy: &AgentLoopGuardPolicy,
    agent_run_context: Option<&AgentRunContext>,
    route_result: Option<&RouteResult>,
    budget_profile: LoopBudgetProfile,
) -> Value {
    let semantic_route_authority = policy.effective_semantic_route_authority();
    let output_contract = route_result.map(|route| &route.output_contract);
    let eligibility = route_result.map(super::migration_class::agent_loop_eligibility);
    let eligible_migration_class = eligibility
        .map(|eligibility| eligibility.compatibility_migration_class())
        .unwrap_or("none");
    let eligibility_bucket = eligibility
        .map(|eligibility| eligibility.bucket_token())
        .unwrap_or("none");
    let eligibility_blocked_reason = eligibility
        .map(|eligibility| eligibility.blocked_reason)
        .unwrap_or("route_unavailable");
    let eligibility_boundary_requirements = eligibility
        .map(|eligibility| eligibility.boundary_requirements)
        .unwrap_or(&[] as &[&str]);
    let selected_migration_class =
        policy.selected_migration_class_for_eligible(eligible_migration_class);
    let agent_loop_selected =
        policy.uses_agent_loop_semantic_authority() && selected_migration_class != "none";
    json!({
        "schema_version": 1,
        "owner_layer": "boundary_layer",
        "semantic_routing": {
            "activation_state": semantic_route_authority.as_str(),
            "ordinary_semantic_authority": if agent_loop_selected {
                "planner_loop_selected_class"
            } else {
                "planner_loop_shadow"
            },
            "normalizer_role": "initial_hint",
            "post_route_role": "boundary_machine_gate",
            "direct_answer_gate_role": "fallback_safety_check",
            "runtime_default_authority": if agent_loop_selected {
                "agent_loop_for_selected_migration_class"
            } else {
                "legacy_pre_agent_until_activation_gates_pass"
            },
            "agent_loop_authority_enabled": agent_loop_selected,
            "chosen_authority": if agent_loop_selected {
                semantic_route_authority.as_str()
            } else {
                "legacy_pre_agent"
            },
            "rollback_reason": if policy.uses_agent_loop_semantic_authority() && selected_migration_class == "none" {
                "migration_class_not_selected"
            } else {
                "none"
            },
        },
        "normalizer_hints": route_result
            .map(normalizer_hints_json)
            .unwrap_or_else(|| json!({
                "schema_version": 1,
                "source": "route_result_machine_fields",
                "available": false,
            })),
        "session": {
            "user_id_present": task.user_id != 0,
            "chat_id_present": task.chat_id != 0,
            "user_key_present": task.user_key.as_deref().is_some_and(|value| !value.trim().is_empty()),
        },
        "workspace": {
            "available": true,
        },
        "capability_visibility": {
            "route_available": route_result.is_some(),
            "visible_skill_candidates_count": route_result
                .map(|route| route.visible_skill_candidates.len())
                .unwrap_or(0),
        },
        "risk": {
            "ceiling": route_result
                .map(|route| route.risk_ceiling.as_str())
                .unwrap_or("unknown"),
        },
        "budget": {
            "profile": budget_profile.as_str(),
            "agent_loop_canary_bucket": policy.agent_loop_canary_bucket.as_str(),
            "eligible_migration_class": eligible_migration_class,
            "selected_migration_class": selected_migration_class,
            "agent_loop_eligibility_bucket": eligibility_bucket,
            "agent_loop_eligibility_blocked_reason": eligibility_blocked_reason,
            "agent_loop_boundary_requirements": eligibility_boundary_requirements,
            "max_rounds": policy.max_rounds,
            "max_steps": policy.max_steps,
            "max_tool_calls": policy.max_tool_calls,
            "no_progress_limit": policy.no_progress_limit,
            "repeat_action_limit": policy.repeat_action_limit,
        },
        "confirmation": {
            "owned_by": "plan_verifier",
        },
        "dry_run": {
            "owned_by": "plan_verifier_execution_adapter",
        },
        "active_bindings": {
            "session_alias_count": agent_run_context
                .map(|ctx| ctx.session_alias_bindings.len())
                .unwrap_or(0),
            "auto_locator_present": agent_run_context
                .and_then(|ctx| ctx.auto_locator_path.as_deref())
                .is_some_and(|value| !value.trim().is_empty()),
            "authoritative_deictic_anchor": agent_run_context
                .map(|ctx| ctx.has_authoritative_deictic_anchor)
                .unwrap_or(false),
            "fuzzy_locator_suggestion_count": agent_run_context
                .map(|ctx| ctx.fuzzy_locator_suggestions.len())
                .unwrap_or(0),
        },
        "memory": {
            "execution_memory_context_present": agent_run_context
                .and_then(|ctx| ctx.memory_context_for_execution.as_deref())
                .is_some_and(|value| !value.trim().is_empty()),
            "cross_turn_recent_execution_context_present": agent_run_context
                .and_then(|ctx| ctx.cross_turn_recent_execution_context.as_deref())
                .is_some_and(|value| !value.trim().is_empty()),
        },
        "delivery_constraints": {
            "delivery_required": route_result
                .map(|route| route.wants_file_delivery || route.output_contract.delivery_required)
                .unwrap_or(false),
            "response_shape": output_contract
                .map(|contract| contract.response_shape.as_str())
                .unwrap_or("unknown"),
            "semantic_kind": output_contract
                .map(|contract| contract.semantic_kind.as_str())
                .unwrap_or("unknown"),
            "locator_kind": output_contract
                .map(|contract| contract.locator_kind.as_str())
                .unwrap_or("unknown"),
            "requires_content_evidence": output_contract
                .map(|contract| contract.requires_content_evidence)
                .unwrap_or(false),
        },
        "pre_agent_gates": pre_agent_gate_summary_json(route_result, agent_run_context),
    })
}

fn normalizer_hints_json(route: &RouteResult) -> Value {
    let contract = &route.output_contract;
    json!({
        "schema_version": 1,
        "source": "route_result_machine_fields",
        "available": true,
        "gate_kind_hint": route.gate_kind().as_str(),
        "route_confidence": route.route_confidence,
        "candidate_contracts": [
            crate::TaskContract::from_route_result(route).compact_prompt_line()
        ],
        "output_contract": {
            "response_shape": contract.response_shape.as_str(),
            "semantic_kind": contract.semantic_kind.as_str(),
            "locator_kind": contract.locator_kind.as_str(),
            "locator_hint_present": !contract.locator_hint.trim().is_empty(),
            "delivery_intent": contract.delivery_intent.as_str(),
            "delivery_required": contract.delivery_required,
            "requires_content_evidence": contract.requires_content_evidence,
        },
        "candidate_locators": normalizer_candidate_locators_json(route),
        "missing_slot_hints": normalizer_missing_slot_hints(route),
        "risk_hints": normalizer_risk_hints(route),
    })
}

fn normalizer_candidate_locators_json(route: &RouteResult) -> Vec<Value> {
    let contract = &route.output_contract;
    let mut locators = Vec::new();
    if contract.locator_kind != crate::OutputLocatorKind::None
        || !contract.locator_hint.trim().is_empty()
    {
        locators.push(json!({
            "kind": contract.locator_kind.as_str(),
            "hint": contract.locator_hint.trim(),
            "hint_present": !contract.locator_hint.trim().is_empty(),
        }));
    }
    locators
}

fn normalizer_missing_slot_hints(route: &RouteResult) -> Vec<&'static str> {
    let contract = &route.output_contract;
    let locator_missing = contract.locator_kind == crate::OutputLocatorKind::None
        && contract.locator_hint.trim().is_empty();
    let mut hints = Vec::new();
    if route.needs_clarify {
        hints.push("clarify_required");
    }
    if locator_missing && (contract.delivery_required || route.wants_file_delivery) {
        hints.push("delivery_locator");
    }
    if locator_missing && contract.requires_content_evidence {
        hints.push("content_evidence_locator");
    }
    hints
}

fn normalizer_risk_hints(route: &RouteResult) -> Vec<&'static str> {
    match route.risk_ceiling {
        crate::RiskCeiling::Unknown => Vec::new(),
        crate::RiskCeiling::Low => vec!["risk_ceiling_low"],
        crate::RiskCeiling::Medium => vec!["risk_ceiling_medium"],
        crate::RiskCeiling::High => vec!["risk_ceiling_high"],
    }
}

fn pre_agent_gate_summary_json(
    route_result: Option<&RouteResult>,
    agent_run_context: Option<&AgentRunContext>,
) -> Value {
    json!({
        "schema_version": 1,
        "intent_normalizer": route_result
            .map(intent_normalizer_initial_hint_json)
            .unwrap_or_else(|| json!({
                "owner_layer": "intent_normalizer",
                "authority_target": "initial_hint_shadow",
                "ownership_class": "semantic_initial_hint",
                "boundary_allowed": false,
                "semantic_migration_target": "planner_loop_decision_envelope",
                "available": false,
            })),
        "post_route_policy": route_result
            .map(|route| post_route_boundary_gate_json(route, agent_run_context))
            .unwrap_or_else(|| json!({
                "owner_layer": "post_route_policy",
                "authority_target": "boundary_machine_gate",
                "ownership_class": "boundary_machine_check",
                "boundary_allowed": true,
                "semantic_migration_target": "none",
                "available": false,
            })),
        "direct_answer_gate": route_result
            .map(direct_answer_fallback_gate_json)
            .unwrap_or_else(|| json!({
                "owner_layer": "direct_answer_gate",
                "authority_target": "fallback_safety_check",
                "ownership_class": "fallback_safety_check",
                "boundary_allowed": true,
                "semantic_migration_target": "planner_loop_decision_envelope",
                "available": false,
            })),
    })
}

fn intent_normalizer_initial_hint_json(route: &RouteResult) -> Value {
    json!({
        "owner_layer": "intent_normalizer",
        "authority_target": "initial_hint_shadow",
        "ownership_class": "semantic_initial_hint",
        "boundary_allowed": false,
        "semantic_migration_target": "planner_loop_decision_envelope",
        "available": true,
        "current_gate_kind": route.gate_kind().as_str(),
        "output_contract_ref": crate::TaskContract::from_route_result(route).compact_prompt_line(),
    })
}

fn post_route_boundary_gate_json(
    route: &RouteResult,
    agent_run_context: Option<&AgentRunContext>,
) -> Value {
    let boundary_class = post_route_boundary_class(route, agent_run_context);
    let boundary_allowed = post_route_boundary_class_is_boundary_owned(boundary_class);
    json!({
        "owner_layer": "post_route_policy",
        "authority_target": "boundary_machine_gate",
        "ownership_class": if boundary_allowed {
            "boundary_machine_check"
        } else {
            "semantic_policy_candidate"
        },
        "boundary_allowed": boundary_allowed,
        "semantic_migration_target": if boundary_allowed {
            "none"
        } else {
            "planner_loop_decision_envelope"
        },
        "available": true,
        "boundary_class": boundary_class,
        "fuzzy_locator_suggestion_count": agent_run_context
            .map(|ctx| ctx.fuzzy_locator_suggestions.len())
            .unwrap_or(0),
        "auto_locator_present": agent_run_context
            .and_then(|ctx| ctx.auto_locator_path.as_deref())
            .is_some_and(|value| !value.trim().is_empty()),
        "delivery_required": route.wants_file_delivery || route.output_contract.delivery_required,
        "requires_content_evidence": route.output_contract.requires_content_evidence,
    })
}

fn post_route_boundary_class(
    route: &RouteResult,
    agent_run_context: Option<&AgentRunContext>,
) -> &'static str {
    if agent_run_context
        .map(|ctx| !ctx.fuzzy_locator_suggestions.is_empty())
        .unwrap_or(false)
    {
        return "locator_fuzzy_candidates";
    }
    if route_reason_has_prefix(route, "clarify_reason_code:missing_")
        || route_reason_has_marker(route, "locator_required_for_path_scoped_content")
        || route_reason_has_marker(route, "deictic_bare_locator_requires_clarify")
        || route_reason_has_marker(route, "unbound_existing_file_delivery_requires_clarify")
        || route_reason_has_marker(route, "unbound_targeted_evidence_requires_clarify")
        || route_reason_has_marker(route, "locatorless_observation_requires_clarify")
    {
        return "locator_binding";
    }
    if route.wants_file_delivery || route.output_contract.delivery_required {
        return "delivery_contract";
    }
    if route.output_contract.requires_content_evidence {
        return "content_evidence_contract";
    }
    "no_boundary_gate_observed"
}

fn post_route_boundary_class_is_boundary_owned(boundary_class: &str) -> bool {
    matches!(
        boundary_class,
        "locator_fuzzy_candidates"
            | "locator_binding"
            | "delivery_contract"
            | "content_evidence_contract"
            | "no_boundary_gate_observed"
    )
}

fn direct_answer_fallback_gate_json(route: &RouteResult) -> Value {
    let boundary_class = direct_answer_gate_boundary_class(route);
    let observed = boundary_class != "not_observed_in_planner_shadow";
    let boundary_allowed = direct_answer_gate_boundary_class_is_boundary_owned(boundary_class);
    json!({
        "owner_layer": "direct_answer_gate",
        "authority_target": "fallback_safety_check",
        "ownership_class": if boundary_allowed {
            "fallback_safety_check"
        } else {
            "semantic_policy_candidate"
        },
        "boundary_allowed": boundary_allowed,
        "semantic_migration_target": if boundary_allowed {
            "none"
        } else {
            "planner_loop_decision_envelope"
        },
        "available": true,
        "observed": observed,
        "boundary_class": boundary_class,
        "observation_class": if observed {
            "legacy_gate_observed"
        } else {
            "not_observed_in_planner_shadow"
        },
    })
}

fn direct_answer_gate_boundary_class(route: &RouteResult) -> &'static str {
    if !route_reason_has_prefix(route, "direct_answer_gate_") {
        return "not_observed_in_planner_shadow";
    }
    if route_reason_has_marker(route, "direct_answer_gate_unbound_deictic_clarify") {
        return "locator_binding_fallback";
    }
    if route_reason_has_marker(route, "direct_answer_gate_bound_candidate_evidence")
        || route_reason_has_marker(route, "direct_answer_gate_recent_count_selection")
    {
        return "evidence_backed_direct_candidate";
    }
    if route_reason_has_marker(route, "direct_answer_gate_memory_update_ignored")
        || route_reason_has_marker(
            route,
            "direct_answer_gate_active_task_text_mutation_ignored",
        )
        || route_reason_has_marker(route, "direct_answer_gate_executionless_promotion_blocked")
        || route_reason_has_marker(route, "direct_answer_gate_existing_observed_result_ignored")
        || route_reason_has_marker(
            route,
            "direct_answer_gate_chat_promotion_without_structured_target_ignored",
        )
        || route_reason_has_marker(
            route,
            "direct_answer_gate_preference_memory_context_ignored",
        )
        || route_reason_has_marker(route, "direct_answer_gate_background_only_ignored")
        || route_reason_has_marker(
            route,
            "direct_answer_gate_exact_candidate_ignored_execution",
        )
        || route_reason_has_marker(
            route,
            "direct_answer_gate_standalone_freeform_clarify_ignored",
        )
    {
        return "fallback_safety_filter";
    }
    if route_reason_has_marker(route, "direct_answer_gate_contract_execute")
        || route_reason_has_marker(route, "direct_answer_gate_inline_transform_execute")
        || route_reason_has_marker(route, "direct_answer_gate_package_manager_detect_execute")
        || route_reason_has_marker(route, "direct_answer_gate_recent_file_context_execute")
        || route_reason_has_marker(route, "direct_answer_gate_artifact_listing_execute")
        || route_reason_has_marker(route, "direct_answer_gate_workspace_child_context_execute")
        || route_reason_has_marker(route, "direct_answer_gate_execute")
    {
        return "semantic_execution_promotion";
    }
    if route_reason_has_marker(route, "direct_answer_gate_clarify") {
        return "semantic_clarify_candidate";
    }
    "legacy_unclassified_gate_observed"
}

fn direct_answer_gate_boundary_class_is_boundary_owned(boundary_class: &str) -> bool {
    matches!(
        boundary_class,
        "not_observed_in_planner_shadow"
            | "locator_binding_fallback"
            | "evidence_backed_direct_candidate"
            | "fallback_safety_filter"
    )
}

fn route_reason_has_marker(route: &RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part == marker)
}

fn route_reason_has_prefix(route: &RouteResult, prefix: &str) -> bool {
    route
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| part.starts_with(prefix))
}

pub(super) async fn run_agent_with_loop(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    let base_policy = load_agent_loop_guard_policy(state);
    let mut loop_state = LoopState::new(base_policy.max_rounds.max(1));
    super::seed_loop_state_from_agent_context(&mut loop_state, agent_run_context);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        initial_execution_recipe_spec(goal, user_text, agent_run_context),
    );
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, route_result);
    let policy = base_policy.adjusted_for_context(loop_state.execution_recipe, route_result);
    loop_state.max_rounds = policy.max_rounds.max(1);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    let enabled_rollout_switches = policy.enabled_rollout_switches();
    if !enabled_rollout_switches.is_empty() {
        loop_state.output_vars.insert(
            "rollout_switches_enabled".to_string(),
            enabled_rollout_switches.join(","),
        );
    }
    maybe_record_agent_decides_shadow_attribution(
        &policy,
        task,
        agent_run_context,
        route_result,
        budget_profile,
        &mut loop_state,
    );
    info!(
        "loop_budget_profile task_id={} profile={} max_rounds={} max_steps={} max_tool_calls={} no_progress_limit={} repeat_action_limit={}",
        task.task_id,
        budget_profile.as_str(),
        policy.max_rounds,
        policy.max_steps,
        policy.max_tool_calls,
        policy.no_progress_limit,
        policy.repeat_action_limit
    );
    let mut round = 1usize;
    let mut answer_verifier_retry_count = 0usize;
    loop {
        while round <= loop_state.max_rounds {
            ensure_task_running(state, task)?;
            loop_state.round_no = round;
            super::maybe_publish_execution_recipe_phase_hint(state, task, &mut loop_state);
            let outcome = run_agent_round(
                state,
                task,
                goal,
                user_text,
                &policy,
                &mut loop_state,
                agent_run_context,
            )
            .await?;
            loop_state.last_stop_signal = outcome.stop_signal.clone();
            if evaluate_round_outcome(task, &mut loop_state, &policy, &outcome) {
                break;
            }
            round += 1;
        }
        let pre_finalize_loop_state = loop_state.clone();
        let mut reply = crate::finalize::finalize_loop_reply(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
        )
        .await?;
        attach_answer_verifier_if_missing(
            state,
            task,
            user_text,
            &policy,
            agent_run_context,
            &mut reply,
        )
        .await;
        let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
        suppress_answer_verifier_retry_if_structurally_satisfied(&mut reply, route_result);
        suppress_answer_verifier_retry_if_user_locator_disambiguation(&mut reply, route_result);
        if try_preserve_rss_source_hosts_from_structured_evidence(route_result, &mut reply) {
            return Ok(reply);
        }
        if try_recover_document_heading_answer_verifier_gap(route_result, &mut reply) {
            return Ok(reply);
        }
        if let Some(verifier) = answer_verifier_retry_summary(&reply, route_result).cloned() {
            if answer_verifier_retry_count < policy.answer_verifier_retry_limit
                && policy.multi_round_enabled
            {
                loop_state = pre_finalize_loop_state;
                answer_verifier_retry_count += 1;
                loop_state.has_recoverable_failure_context = true;
                loop_state.delivery_messages.clear();
                loop_state.last_user_visible_respond = None;
                loop_state.last_publishable_synthesis_output = None;
                loop_state.last_stop_signal = Some("answer_verifier_retry".to_string());
                attempt_ledger::record_attempt_with_retry_instruction(
                    &mut loop_state,
                    "answer_verifier",
                    &format!(
                        "missing_evidence_fields={}",
                        verifier.missing_evidence_fields.join(",")
                    ),
                    crate::executor::StepExecutionStatus::Error,
                    &reply.text,
                    Some("answer_incomplete"),
                    &verifier.answer_incomplete_reason,
                    Some(&verifier.retry_instruction),
                );
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n("telegram.progress.answer_incomplete_retry", &[]),
                );
                if loop_state.round_no >= loop_state.max_rounds {
                    loop_state.max_rounds += 1;
                }
                round = loop_state.round_no + 1;
                info!(
                    "loop_round_extend task_id={} round={} reason=answer_verifier_retry new_max_rounds={}",
                    task.task_id, loop_state.round_no, loop_state.max_rounds
                );
                continue;
            }
            warn!(
                "answer_verifier_retry_exhausted task_id={} retry_count={} limit={} reason={}",
                task.task_id,
                answer_verifier_retry_count,
                policy.answer_verifier_retry_limit,
                crate::truncate_for_log(&verifier.answer_incomplete_reason)
            );
            if try_recover_log_analyze_answer_verifier_gap(user_text, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_count_answer_verifier_gap(route_result, user_text, &mut reply)
            {
                return Ok(reply);
            }
            if try_recover_structured_search_answer_verifier_gap(
                route_result,
                user_text,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_rss_news_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_document_heading_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_scalar_output_format_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_http_health_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_local_health_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_generic_path_content_read_range_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_content_excerpt_summary_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            mark_reply_failed_after_answer_verifier_exhausted(user_text, &mut reply, &verifier);
        }
        return Ok(reply);
    }
}

async fn attach_answer_verifier_if_missing(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    agent_run_context: Option<&AgentRunContext>,
    reply: &mut AskReply,
) {
    if reply.should_fail_task || reply_final_status_is_clarify(reply) {
        return;
    }
    let Some(route_result) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    let Some(journal) = reply.task_journal.as_mut() else {
        return;
    };
    if journal.answer_verifier_summary.is_some() {
        return;
    }
    if let Some((selected_class, answer_verifier)) =
        selected_contract_structured_evidence_gap(policy, route_result, journal)
    {
        journal.record_answer_verifier_summary(answer_verifier);
        let summary = journal.answer_verifier_summary.as_ref();
        journal.rollout_attribution.push(
            crate::task_journal::TaskJournalRolloutAttribution::selected_contract_structured_evidence_block(
                summary,
                selected_class,
            ),
        );
        return;
    }
    if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route_result,
        journal,
        &reply.text,
    )
    .await
    {
        journal.record_answer_verifier_summary(answer_verifier);
    }
}

fn selected_contract_structured_evidence_gap(
    policy: &AgentLoopGuardPolicy,
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Option<(&'static str, crate::answer_verifier::AnswerVerifierOut)> {
    if !policy.structured_evidence_required_for_selected_contracts {
        return None;
    }
    let selected_class =
        super::agent_loop_authority_selected_migration_class_for_policy(policy, route_result)?;
    crate::answer_verifier::local_missing_evidence_verifier_gap(route_result, journal)
        .map(|gap| (selected_class, gap))
}

#[cfg(test)]
#[path = "loop_control_authority_tests.rs"]
mod authority_tests;
#[cfg(test)]
#[path = "loop_control_tests.rs"]
mod tests;
