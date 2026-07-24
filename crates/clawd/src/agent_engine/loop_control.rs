use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::{info, warn};

use super::support::publish_agent_loop_checkpoint_progress;
use super::{
    attempt_ledger, ensure_task_running, execute_actions_once, load_agent_loop_guard_policy,
    prepare_round_actions, push_round_trace, verifier_confirmation_gate_requires_checkpoint,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, AskReply, ClaimedTask, IntentOutputContract};

#[path = "loop_control_answer_recovery.rs"]
mod loop_control_answer_recovery;
#[path = "loop_control_finalization_gate.rs"]
mod loop_control_finalization_gate;
#[path = "loop_control_observe_round.rs"]
mod loop_control_observe_round;
#[path = "loop_control_plan_verifier_recovery.rs"]
mod loop_control_plan_verifier_recovery;
#[path = "loop_control_post_write_evidence_guard.rs"]
mod loop_control_post_write_evidence_guard;
#[path = "loop_control_structured_clarify.rs"]
mod loop_control_structured_clarify;
#[path = "loop_control_verifier_retry_commit.rs"]
mod loop_control_verifier_retry_commit;

use loop_control_answer_recovery::*;
use loop_control_finalization_gate::*;
pub(in crate::agent_engine) use loop_control_observe_round::observation_round_needs_planner;
use loop_control_observe_round::{
    observe_only_round_should_continue, read_observe_round_should_continue,
};
pub(in crate::agent_engine) use loop_control_plan_verifier_recovery::plan_verifier_rejection_is_repairable;
use loop_control_plan_verifier_recovery::*;
use loop_control_post_write_evidence_guard::*;
use loop_control_structured_clarify::*;
use loop_control_verifier_retry_commit::verifier_retry_answer_has_required_machine_evidence;

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

fn answer_verifier_summary_to_out(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> crate::answer_verifier::AnswerVerifierOut {
    crate::answer_verifier::AnswerVerifierOut {
        pass: verifier.pass,
        missing_evidence_fields: verifier.missing_evidence_fields.clone(),
        answer_incomplete_reason: verifier.answer_incomplete_reason.clone(),
        should_retry: verifier.should_retry,
        retry_instruction: verifier.retry_instruction.clone(),
        confidence: verifier.confidence,
    }
    .normalized()
}

fn commit_answer_verifier_retry_answer(reply: &mut AskReply, retried_answer: String) -> bool {
    if !verifier_retry_answer_has_required_machine_evidence(reply, &retried_answer) {
        info!("answer_verifier_retry_commit_rejected_missing_machine_validation_evidence");
        return false;
    }
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(retried_answer.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&retried_answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.record_final_stop_signal(
            crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
        );
    }
    reply.text = retried_answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = true;
    true
}

async fn try_bounded_answer_verifier_synthesis_retry(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    route: &crate::answer_verifier::AnswerContract,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
    reply: &mut AskReply,
) -> bool {
    let Some(journal_snapshot) = reply.task_journal.clone() else {
        return false;
    };
    let verifier_out = answer_verifier_summary_to_out(verifier);
    let Some(retried_answer) = crate::finalize::retry_loop_answer_after_verifier(
        state,
        task,
        user_text,
        &route.output_contract,
        &journal_snapshot,
        &reply.text,
        &verifier_out,
    )
    .await
    else {
        return false;
    };
    let retry_verifier = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route,
        &journal_snapshot,
        &retried_answer,
    )
    .await;
    if let Some(retry_verifier) = retry_verifier {
        if !retry_verifier_accepts_rewritten_answer(&retry_verifier, &retried_answer) {
            if let Some(journal) = reply.task_journal.as_mut() {
                journal.record_answer_verifier_summary(retry_verifier);
            }
            return false;
        }
    } else if !retry_rewritten_answer_is_publishable(&retried_answer) {
        return false;
    }
    commit_answer_verifier_retry_answer(reply, retried_answer)
}

async fn record_session_start_hooks(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
) {
    let session_start = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::SessionStart,
        "agent_loop.session_start",
        json!({
            "task_kind": task.kind,
            "task_channel": task.channel,
        }),
    )
    .await;
    loop_state
        .task_observations
        .extend(session_start.machine_observations("agent_loop"));

    let prompt_submit = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::UserPromptSubmit,
        "agent_loop.user_prompt_submit",
        json!({
            "input_char_count": user_text.chars().count(),
            "input_byte_count": user_text.len(),
        }),
    )
    .await;
    loop_state
        .task_observations
        .extend(prompt_submit.machine_observations("agent_loop"));
}

fn terminal_user_answer_stop_signal(loop_state: &LoopState) -> Option<&'static str> {
    has_authoritative_delivery(loop_state).then_some("terminal_user_answer_ready")
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

fn route_expects_terminal_user_answer(route_result: &IntentOutputContract) -> bool {
    if route_result.delivery_required {
        return false;
    }
    !matches!(
        route_result.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
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
                | AgentAction::CallCapability { .. }
                | AgentAction::SynthesizeAnswer { .. }
        )
    })
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

fn route_needs_workspace_text_evidence_before_observed_finalize(
    route: &IntentOutputContract,
) -> bool {
    route.requires_content_evidence
        && !route.delivery_required
        && route.response_shape == crate::OutputResponseShape::Free
        && route.does_not_request_exact_command_output()
        && route.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.locator_hint.trim().is_empty()
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

fn structured_field_selector_observation_can_finalize(
    route_result: &IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    crate::finalize::strict_capability_projection_ready(route_result, loop_state)
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
    let route_result = loop_state
        .output_contract
        .as_ref()
        .or_else(|| agent_run_context.and_then(|ctx| ctx.output_contract()));
    let Some(route_result) = route_result else {
        return false;
    };
    let output_contract = route_result.clone();
    if false || !loop_state.has_tool_or_skill_output || has_authoritative_delivery(loop_state) {
        return false;
    }
    if structured_field_selector_observation_can_finalize(route_result, loop_state) {
        return true;
    }
    if route_result.requests_path_inspection() {
        return false;
    }
    if route_needs_workspace_text_evidence_before_observed_finalize(route_result)
        && !has_discussion_followup_action(actions)
        && !last_executable_action(actions).is_some_and(action_reads_text_content)
    {
        return false;
    }
    let has_direct_observed_answer =
        super::observed_output::extract_answer_from_observed_output(loop_state, agent_run_context)
            .is_some();
    let has_observed_stop_candidate = has_direct_observed_answer
        || super::observed_output::has_observed_answer_candidates(loop_state);
    if read_observe_round_should_continue(&output_contract, loop_state, actions) {
        return false;
    }
    if observation_round_needs_planner(&output_contract, loop_state, actions)
        && !has_observed_stop_candidate
    {
        return false;
    }
    if super::observed_output::route_disallows_direct_observation_passthrough(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if route_result.response_shape != crate::OutputResponseShape::Scalar
        && latest_path_batch_facts_all_missing(loop_state)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if has_direct_observed_answer
        && route_result.response_shape != crate::OutputResponseShape::Scalar
    {
        return true;
    }
    if route_result.response_shape == crate::OutputResponseShape::Scalar {
        if super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return true;
        }
        if super::observed_output::scalar_route_prefers_structured_observed_answer(
            route_result,
            loop_state,
        ) && super::observed_output::extract_answer_from_observed_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return true;
        }
    }
    let can_stop = has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && has_observed_stop_candidate;
    can_stop
}

fn coding_workflow_ready_for_model_finalization(loop_state: &LoopState) -> bool {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "agent-loop-coding-finalization-probe",
        "ask",
        "",
    );
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    for observation in &loop_state.task_observations {
        journal.push_task_observation(observation.clone());
    }
    let summary = journal.to_summary_json();
    let Some(workflow) = summary.get("coding_workflow") else {
        return false;
    };
    let has_changes = workflow
        .get("changed_file_count")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0);
    let journal_verification_ready = workflow.get("verification_status").and_then(Value::as_str)
        == Some("verified")
        && workflow.get("next_step").and_then(Value::as_str) == Some("summarize")
        && workflow
            .pointer("/validation_gate/gate_status")
            .and_then(Value::as_str)
            == Some("satisfied")
        && workflow
            .pointer("/validation_gate/can_report_fully_verified")
            .and_then(Value::as_bool)
            == Some(true);
    let latest_command_validation_ready =
        loop_state
            .latest_validation_result
            .as_ref()
            .is_some_and(|validation| {
                validation.get("status").and_then(Value::as_str) == Some("passed")
                    && validation.get("verification_scope").and_then(Value::as_str)
                        == Some("command")
            });
    has_changes && (journal_verification_ready || latest_command_validation_ready)
}

fn loop_state_has_checkpoint_handoff(loop_state: &LoopState) -> bool {
    let Some(lifecycle) = loop_state.task_lifecycle.as_ref() else {
        return false;
    };
    let lifecycle_state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(lifecycle_state, "waiting" | "background" | "needs_user") {
        return false;
    }
    let lifecycle_checkpoint_id = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let checkpoint_id = loop_state
        .task_checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    matches!(
        (lifecycle_checkpoint_id, checkpoint_id),
        (Some(lifecycle_id), Some(checkpoint_id)) if lifecycle_id == checkpoint_id
    )
}

fn checkpoint_handoff_reply(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> AskReply {
    let mut journal = crate::finalize::build_from_loop_state(
        task,
        user_text,
        loop_state,
        agent_run_context,
        None,
        true,
        "",
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    journal.final_answer = None;
    journal.final_status = None;
    journal.final_stop_signal = None;
    AskReply::non_llm(String::new()).with_task_journal(journal)
}

fn recoverable_provider_blocker_resume_reason(loop_state: &LoopState) -> Option<&'static str> {
    use claw_core::provider_failure_policy::{
        PROVIDER_WAIT_RECOVERY_ACTION, PROVIDER_WAIT_RESUME_REASON,
    };

    let latest = loop_state
        .attempt_ledger_entries
        .iter()
        .rev()
        .find(|entry| {
            entry.provider_status.is_some()
                && entry.recovery_action.trim() == PROVIDER_WAIT_RECOVERY_ACTION
                && entry.status.trim() != crate::executor::StepExecutionStatus::Ok.as_str()
        })?;
    latest.provider_status.as_ref()?;
    Some(PROVIDER_WAIT_RESUME_REASON)
}

fn worker_soft_checkpoint_after_seconds(worker_timeout_secs: u64) -> Option<u64> {
    let timeout = worker_timeout_secs.max(1);
    if timeout <= 2 {
        return None;
    }
    let reserve = (timeout / 10).clamp(1, 30);
    let soft_after = timeout.saturating_sub(reserve);
    (soft_after > 0 && soft_after < timeout).then_some(soft_after)
}

fn worker_soft_checkpoint_after(worker_timeout_secs: u64) -> Option<Duration> {
    worker_soft_checkpoint_after_seconds(worker_timeout_secs).map(Duration::from_secs)
}

fn task_budget_soft_slice_exhausted(started_at: Instant, loop_state: &LoopState) -> bool {
    loop_state
        .task_budget_slice
        .as_ref()
        .is_some_and(|slice| started_at.elapsed().as_millis() >= u128::from(slice.soft_slice_ms))
}

fn loop_state_has_recoverable_checkpoint_state(loop_state: &LoopState) -> bool {
    loop_state.task_checkpoint.is_some()
        || !loop_state.executed_step_results.is_empty()
        || !loop_state.successful_action_fingerprints.is_empty()
        || loop_state.has_tool_or_skill_output
}

fn task_budget_profile(
    profile: super::support::LoopBudgetProfile,
) -> crate::task_budget_contract::TaskBudgetProfile {
    use crate::task_budget_contract::TaskBudgetProfile;
    match profile {
        super::support::LoopBudgetProfile::General => TaskBudgetProfile::General,
        super::support::LoopBudgetProfile::FastRead => TaskBudgetProfile::FastRead,
        super::support::LoopBudgetProfile::GroundedSummary => TaskBudgetProfile::GroundedSummary,
        super::support::LoopBudgetProfile::MultiStepWorkspace => {
            TaskBudgetProfile::MultiStepWorkspace
        }
        super::support::LoopBudgetProfile::OpsClosedLoop => TaskBudgetProfile::OpsClosedLoop,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChildLoopBudgetLimits {
    max_rounds: u64,
    max_tool_calls: u64,
    timeout_ms: u64,
}

fn child_loop_budget_limits(payload_json: &str) -> Option<ChildLoopBudgetLimits> {
    let payload = serde_json::from_str::<Value>(payload_json).ok()?;
    if payload.get("task_role").and_then(Value::as_str) != Some("subagent_child") {
        return None;
    }
    let budget = payload.pointer("/child_task_contract/budget")?;
    Some(ChildLoopBudgetLimits {
        max_rounds: budget.get("max_rounds")?.as_u64()?.clamp(1, 256),
        max_tool_calls: budget.get("max_tool_calls")?.as_u64()?.clamp(1, 512),
        timeout_ms: budget.get("timeout_ms")?.as_u64()?.clamp(1_000, 86_400_000),
    })
}

fn clamp_child_loop_guard_policy(task: &ClaimedTask, policy: &mut AgentLoopGuardPolicy) {
    let Some(limits) = child_loop_budget_limits(&task.payload_json) else {
        return;
    };
    policy.max_actions_per_turn = policy
        .max_actions_per_turn
        .min(limits.max_tool_calls as usize)
        .max(1);
}

fn clamp_child_task_budget_policy(
    task: &ClaimedTask,
    policy: &mut crate::task_budget_contract::TaskBudgetPolicy,
) {
    let Some(limits) = child_loop_budget_limits(&task.payload_json) else {
        return;
    };
    policy.hard_ceilings.model_turns = policy.hard_ceilings.model_turns.min(limits.max_rounds);
    policy.hard_ceilings.tool_calls = policy.hard_ceilings.tool_calls.min(limits.max_tool_calls);
    policy.hard_ceilings.elapsed_ms = policy.hard_ceilings.elapsed_ms.min(limits.timeout_ms);
}

fn initialize_task_budget_slice(
    loop_state: &mut LoopState,
    profile: super::support::LoopBudgetProfile,
    task_budget_policy: &crate::task_budget_contract::TaskBudgetPolicy,
    soft_checkpoint_after: Option<Duration>,
    worker_timeout_seconds: u64,
) {
    let soft_slice_ms = soft_checkpoint_after
        .unwrap_or_else(|| Duration::from_secs(worker_timeout_seconds.max(1)))
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    loop_state.task_budget_worker_soft_limit_ms = soft_slice_ms;
    if let Some(slice) = loop_state.task_budget_slice.as_mut() {
        slice.soft_slice_ms = slice.soft_slice_ms.min(soft_slice_ms).max(1);
        return;
    }
    let profile = task_budget_profile(profile);
    let mut profile_policy = task_budget_policy.profile(profile);
    profile_policy.soft_slice_ms = profile_policy.soft_slice_ms.min(soft_slice_ms);
    loop_state.task_budget_slice = Some(
        crate::task_budget_contract::TaskBudgetSlice::new_with_policy(
            profile,
            profile_policy,
            task_budget_policy.hard_ceilings.clone(),
        ),
    );
}

fn apply_task_budget_profile(
    loop_state: &mut LoopState,
    task_budget_policy: &crate::task_budget_contract::TaskBudgetPolicy,
    profile: crate::task_budget_contract::TaskBudgetProfile,
) {
    if let Some(slice) = loop_state.task_budget_slice.as_mut() {
        slice.apply_profile(
            profile,
            task_budget_policy.profile(profile),
            loop_state.task_budget_worker_soft_limit_ms,
        );
    }
}

fn verified_action_effect(
    state: &AppState,
    action: &AgentAction,
) -> crate::execution_recipe::ActionEffect {
    let resolved =
        crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
    match resolved {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            crate::execution_recipe::classify_skill_action_effect(state, &tool, &args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => crate::execution_recipe::ActionEffect::default(),
    }
}

fn budget_profile_for_prepared_round(
    state: &AppState,
    loop_state: &LoopState,
    prepared_round: &super::prepare_round::PreparedRoundActions,
) -> crate::task_budget_contract::TaskBudgetProfile {
    let mut facts = crate::task_budget_contract::VerifiedPlanBudgetFacts {
        needs_confirmation: prepared_round.verify_result.needs_confirmation,
        has_continuation: matches!(
            task_budget_lifecycle_state(loop_state),
            Some("waiting" | "background")
        ) || loop_state
            .task_checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.get("pending_async_job"))
            .is_some_and(|value| !value.is_null()),
        ops_closed_loop: matches!(
            loop_state.execution_recipe.kind,
            crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
        ),
        ..crate::task_budget_contract::VerifiedPlanBudgetFacts::default()
    };
    if let Some(contract) = prepared_round.effective_output_contract.as_ref() {
        facts.evidence_required = contract.requires_content_evidence
            || !crate::evidence_policy::required_evidence_fields_for_output_contract(contract)
                .is_empty();
        facts.delivery_required = contract.delivery_required;
    }
    for action in &prepared_round.actions {
        if matches!(
            action,
            AgentAction::CallTool { .. }
                | AgentAction::CallSkill { .. }
                | AgentAction::CallCapability { .. }
        ) {
            facts.action_count = facts.action_count.saturating_add(1);
        }
        let effect = verified_action_effect(state, action);
        facts.observe_count += usize::from(effect.observes);
        facts.mutate_count += usize::from(effect.mutates);
        facts.validate_count += usize::from(effect.validates);
    }
    crate::task_budget_contract::profile_for_verified_plan(facts)
}

fn select_round_task_budget_profile(
    current_profile: Option<crate::task_budget_contract::TaskBudgetProfile>,
    candidate_profile: crate::task_budget_contract::TaskBudgetProfile,
) -> (crate::task_budget_contract::TaskBudgetProfile, bool) {
    let Some(current_profile) = current_profile else {
        return (candidate_profile, true);
    };
    let profile = current_profile.widen_with(candidate_profile);
    (profile, profile != current_profile)
}

fn task_budget_lifecycle_state(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .task_lifecycle
        .as_ref()
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
}

fn round_requests_continuation(round: &RoundOutcome) -> bool {
    round.stop_signal.as_deref().is_some_and(|signal| {
        matches!(
            signal,
            "recoverable_failure_continue_round"
                | "capability_groups_loaded"
                | "replan_from_verifier_signal"
                | "post_write_validation_reserve"
                | "repeat_action_limit"
                | "repeat_completed_action"
                | "structured_observation_already_ready"
        )
    })
}

fn round_model_finished(outcome: Option<&RoundOutcome>) -> bool {
    outcome.is_some_and(|round| {
        !round_requests_continuation(round)
            && (round.executed_actions == 0 || round.stop_signal.is_some())
    })
}

fn round_is_policy_terminal(outcome: Option<&RoundOutcome>) -> bool {
    outcome.is_some_and(|round| {
        round.had_error || round.stop_signal.as_deref() == Some("repeat_action_limit")
    })
}

fn round_action_budget_metrics(state: &AppState, loop_state: &LoopState) -> (usize, usize) {
    let Some(plan) = loop_state
        .round_traces
        .last()
        .and_then(|round| round.plan_result.as_ref())
    else {
        return (0, 0);
    };
    let planned_action_count = plan
        .steps
        .iter()
        .filter(|step| step.action_type != "think")
        .count();
    let independent_read_count = plan
        .steps
        .iter()
        .filter(|step| step.depends_on.is_empty())
        .filter_map(crate::PlanStep::to_agent_action)
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallCapability { .. }
                    | AgentAction::CallTool { .. }
                    | AgentAction::CallSkill { .. }
            ) && !verified_action_effect(state, action).mutates
        })
        .count();
    (
        planned_action_count,
        usize::from(independent_read_count > 1) * independent_read_count,
    )
}

fn budget_replan_cause<'a>(
    decision: crate::task_budget_contract::BudgetDecision,
    outcome: Option<&'a RoundOutcome>,
) -> Option<&'a str> {
    if decision != crate::task_budget_contract::BudgetDecision::Continue {
        return None;
    }
    Some(
        outcome
            .and_then(|round| round.stop_signal.as_deref())
            .unwrap_or("round_observation"),
    )
}

fn next_resumable_budget_action(
    decision: crate::task_budget_contract::BudgetDecision,
    lifecycle_state: Option<&str>,
) -> Option<&'static str> {
    use crate::task_budget_contract::BudgetDecision;
    match decision {
        BudgetDecision::CheckpointRequeue => Some("resume_checkpoint"),
        BudgetDecision::Waiting if lifecycle_state == Some("background") => Some("poll_async_job"),
        BudgetDecision::Waiting => Some("resume_when_ready"),
        BudgetDecision::NeedsUser => Some("await_user_input"),
        BudgetDecision::Continue | BudgetDecision::Finish | BudgetDecision::Terminal => None,
    }
}

fn observe_task_budget(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    outcome: Option<&RoundOutcome>,
    loop_started_at: Instant,
    soft_slice_exhausted: bool,
) -> crate::task_budget_contract::BudgetDecision {
    use crate::task_budget_contract::{BudgetObservation, BudgetProgress};

    let unique_successful_actions = loop_state.successful_action_fingerprints.len() as u64;
    let artifact_count = super::progress_contract::unique_artifact_count(loop_state) as u64;
    let lifecycle_state = task_budget_lifecycle_state(loop_state).map(str::to_string);
    let provider_waiting = outcome.is_some_and(|round| {
        round.stop_signal.as_deref() == Some("recoverable_failure_continue_round")
    }) && recoverable_provider_blocker_resume_reason(loop_state).is_some();
    let progress = BudgetProgress {
        evidence_count: super::progress_contract::machine_progress_fingerprint_count(loop_state)
            as u64,
        machine_progress_digest: super::progress_contract::machine_progress_digest(loop_state),
        artifact_count,
        completed_plan_nodes: unique_successful_actions,
        verified_state_transitions: u64::from(loop_state.latest_validation_result.is_some()),
        async_continuations: u64::from(lifecycle_state.as_deref() == Some("background")),
        stagnation_count: loop_state.consecutive_no_progress.min(u32::MAX as usize) as u32,
    };
    let policy_terminal = round_is_policy_terminal(outcome);
    let model_finished = round_model_finished(outcome);
    let resumable = loop_state_has_recoverable_checkpoint_state(loop_state);
    let cumulative_elapsed_ms = loop_state.task_budget_slice_base_elapsed_ms.saturating_add(
        loop_started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64,
    );
    let cost = state.task_llm_cost_summary(&task.task_id);
    let stagnation_tolerance = loop_state
        .task_budget_slice
        .as_ref()
        .map(|slice| slice.stagnation_tolerance)
        .unwrap_or(1);
    let observation = BudgetObservation {
        cumulative_model_turns: state.task_llm_call_count(&task.task_id) as u64,
        cumulative_tool_calls: loop_state.tool_calls_total as u64,
        cumulative_input_tokens: cost.input_tokens,
        cumulative_output_tokens: cost.output_tokens,
        cumulative_cost_usd_nanos: cost.estimated_cost_usd_nanos,
        cumulative_elapsed_ms,
        progress,
        model_finished,
        needs_user: loop_state.pending_user_input_required
            || lifecycle_state.as_deref() == Some("needs_user"),
        waiting: provider_waiting
            || matches!(lifecycle_state.as_deref(), Some("waiting" | "background")),
        cancelled: false,
        policy_terminal,
        stagnation_exhausted: loop_state.consecutive_no_progress >= stagnation_tolerance as usize,
        resumable,
        soft_slice_exhausted,
    };
    let (planned_action_count, independently_batchable_count) = outcome
        .map(|_| round_action_budget_metrics(state, loop_state))
        .unwrap_or_default();
    let Some(slice) = loop_state.task_budget_slice.as_mut() else {
        return crate::task_budget_contract::BudgetDecision::Terminal;
    };
    let decision = slice.observe(observation);
    let event_payload = json!({
        "schema_version": 1,
        "decision": decision.as_str(),
        "profile": slice.profile.as_str(),
        "soft_slice_ms": slice.soft_slice_ms,
        "continuation_index": slice.continuation_index,
        "cumulative_model_turns": slice.cumulative_model_turns,
        "cumulative_tool_calls": slice.cumulative_tool_calls,
        "cumulative_input_tokens": slice.cumulative_input_tokens,
        "cumulative_output_tokens": slice.cumulative_output_tokens,
        "cumulative_cost_usd_nanos": slice.cumulative_cost_usd_nanos,
        "cumulative_elapsed_ms": slice.cumulative_elapsed_ms,
        "stagnation_tolerance": slice.stagnation_tolerance,
        "provider_timeout_class": slice.provider_timeout_class.as_str(),
        "tool_timeout_class": slice.tool_timeout_class.as_str(),
        "progress": slice.progress,
        "observed_progress": slice.progress.observed_progress(),
        "soft_slice_exhausted": soft_slice_exhausted,
        "resumable": resumable,
        "planned_action_count": planned_action_count,
        "independently_batchable_count": independently_batchable_count,
        "executed_action_count": outcome.map_or(0, |round| round.executed_actions),
        "round_stop_signal": outcome.and_then(|round| round.stop_signal.as_deref()),
        "replan_cause": budget_replan_cause(decision, outcome),
        "next_resumable_action": next_resumable_budget_action(decision, lifecycle_state.as_deref()),
        "hard_model_turns": slice.hard_ceilings.model_turns,
        "hard_tool_calls": slice.hard_ceilings.tool_calls,
        "hard_total_tokens": slice.hard_ceilings.total_tokens,
        "hard_cost_usd_nanos": slice.hard_ceilings.cost_usd_nanos,
        "hard_elapsed_ms": slice.hard_ceilings.elapsed_ms,
        "hard_continuations": slice.hard_ceilings.continuations,
        "hard_non_resumable_tool_runtime_ms": slice.hard_ceilings.non_resumable_tool_runtime_ms,
    });
    if let Err(err) = crate::task_event_transport::publish_claimed_event(
        state,
        task,
        "budget_decision",
        event_payload,
    ) {
        warn!(
            task_id = task.task_id,
            claim_attempt = task.claim_attempt,
            error = %err,
            "task_budget_decision_event_publish_failed"
        );
    }
    decision
}

async fn run_agent_round(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &mut AgentLoopGuardPolicy,
    task_budget_policy: &crate::task_budget_contract::TaskBudgetPolicy,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    initial_plan: Option<&crate::PlanResult>,
) -> Result<RoundOutcome, String> {
    info!(
        "loop_round_start task_id={} round={} total_steps={} tool_calls_total={}",
        task.task_id,
        loop_state.round_no,
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
        initial_plan,
    )
    .await?;
    push_round_trace(loop_state, goal, &prepared_round);
    for resolution in &prepared_round.verify_result.capability_resolutions {
        let step_in_round = resolution.plan_step_index + 1;
        let mut observation = resolution.record.dispatch_observation(
            loop_state.round_no,
            loop_state.total_steps_executed + step_in_round,
            step_in_round,
        );
        if let Some(object) = observation.as_object_mut() {
            object.insert(
                "plan_step_id".to_string(),
                serde_json::Value::String(resolution.plan_step_id.clone()),
            );
            object.insert(
                "resolution_stage".to_string(),
                serde_json::Value::String("verify".to_string()),
            );
        }
        loop_state.task_observations.push(observation);
    }
    crate::task_event_transport::publish_loop_state_snapshot(state, task, user_text, loop_state);
    record_agent_loop_decision_envelope_output_vars(loop_state, &prepared_round.plan_result);
    let first_verified_plan_profile = !loop_state
        .output_vars
        .contains_key("agent_loop.planner_budget_profile");
    let guard_profile = AgentLoopGuardPolicy::budget_profile_for_context(
        loop_state.execution_recipe,
        prepared_round.effective_output_contract.as_ref(),
    );
    let candidate_profile = budget_profile_for_prepared_round(state, loop_state, &prepared_round);
    let current_profile = (!first_verified_plan_profile)
        .then(|| {
            loop_state
                .task_budget_slice
                .as_ref()
                .map(|slice| slice.profile)
        })
        .flatten();
    let (profile, profile_changed) =
        select_round_task_budget_profile(current_profile, candidate_profile);
    *policy = policy.adjusted_for_task_budget_profile(profile);
    clamp_child_loop_guard_policy(task, policy);
    apply_task_budget_profile(loop_state, task_budget_policy, profile);
    loop_state.output_vars.insert(
        "agent_loop.planner_budget_profile".to_string(),
        profile.as_str().to_string(),
    );
    if profile_changed {
        info!(
            "loop_planner_budget_profile task_id={} profile={} candidate_profile={} max_actions_per_turn={} soft_slice_ms={} stagnation_tolerance={}",
            task.task_id,
            profile.as_str(),
            candidate_profile.as_str(),
            policy.max_actions_per_turn,
            loop_state
                .task_budget_slice
                .as_ref()
                .map(|slice| slice.soft_slice_ms)
                .unwrap_or_default(),
            loop_state
                .task_budget_slice
                .as_ref()
                .map(|slice| slice.stagnation_tolerance)
                .unwrap_or_default()
        );
        info!(
            task_id = task.task_id,
            guard_profile = guard_profile.as_str(),
            task_budget_profile = profile.as_str(),
            task_budget_candidate_profile = candidate_profile.as_str(),
            "loop_structured_budget_profile_selected"
        );
    }
    if let Some(output_contract) = prepared_round.effective_output_contract.as_ref() {
        loop_state.output_contract = Some(output_contract.clone());
        if let Some(final_answer_shape) =
            crate::evidence_policy::final_answer_shape_for_output_contract(output_contract)
        {
            loop_state.output_vars.insert(
                "agent_loop.final_answer_shape".to_string(),
                final_answer_shape.as_str().to_string(),
            );
            loop_state.output_vars.insert(
                "agent_loop.final_answer_shape_class".to_string(),
                final_answer_shape.class().as_str().to_string(),
            );
        }
    }
    if let Some(outcome) =
        recover_plan_verifier_rejection(loop_state, &prepared_round.verify_result)
    {
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            outcome.next_goal_hint.as_deref().unwrap_or("")
        );
        return Ok(outcome);
    }
    if let Some(outcome) = recover_run_cmd_confirmation_with_scoped_capability_replan(
        loop_state,
        &prepared_round.verify_result,
    ) {
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            outcome.next_goal_hint.as_deref().unwrap_or("")
        );
        return Ok(outcome);
    }
    if verifier_confirmation_gate_requires_checkpoint(&prepared_round.verify_result) {
        let outcome = RoundOutcome {
            executed_actions: 0,
            had_error: false,
            stop_signal: Some("confirmation_required".to_string()),
            next_goal_hint: Some("await_explicit_confirmation".to_string()),
            no_progress: false,
        };
        info!(
            "loop_round_eval task_id={} round={} executed_actions=0 no_progress=false stop_signal=confirmation_required next_goal_hint=await_explicit_confirmation",
            task.task_id, loop_state.round_no
        );
        return Ok(outcome);
    }
    if let Some(intent) = structured_respond_terminal_intent_from_plan(&prepared_round.plan_result)
        .filter(|intent| intent.terminal_intent == "clarify")
        .filter(|_| actions_allow_structured_respond_terminal_intent(&prepared_round.actions))
        .or_else(|| {
            structured_respond_terminal_intent_from_boundary_observation_clarify(
                loop_state,
                &prepared_round.actions,
            )
        })
    {
        if let Some(outcome) = try_recover_inconsistent_boundary_clarify(
            loop_state,
            prepared_round.effective_output_contract.as_ref(),
            &intent,
        ) {
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
    if let Some(intent) = forced_boundary_observation_clarify_intent(loop_state, &actions) {
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
    loop_state.verified_action_window_active =
        prepared_round.verify_result.approved && !actions.is_empty();
    loop_state.active_verified_actions = actions.clone();
    let execute_result = execute_actions_once(
        state,
        task,
        goal,
        user_text,
        &actions,
        loop_state,
        policy,
        agent_run_context,
    )
    .await;
    loop_state.verified_action_window_active = false;
    loop_state.active_verified_actions.clear();
    let mut outcome = execute_result?;
    if outcome.stop_signal.is_none() {
        if let Some(stop_signal) = terminal_user_answer_stop_signal(loop_state) {
            outcome.stop_signal = Some(stop_signal.to_string());
        }
    }
    if outcome.stop_signal.is_none() && coding_workflow_ready_for_model_finalization(loop_state) {
        outcome.stop_signal = Some("verified_workflow_ready_for_synthesis".to_string());
    }
    if outcome.stop_signal.is_none()
        && should_stop_for_observed_finalize(agent_run_context, loop_state, &actions)
    {
        outcome.stop_signal = Some("observed_output_ready".to_string());
    }
    if outcome.stop_signal.is_none() {
        let output_contract = loop_state.output_contract.clone().or_else(|| {
            agent_run_context
                .and_then(|ctx| ctx.output_contract())
                .cloned()
        });
        if output_contract.as_ref().is_some_and(|contract| {
            crate::finalize::record_strict_capability_projection_issue(contract, loop_state)
        }) {
            outcome.next_goal_hint = Some("resolve_generic_projection_issue".to_string());
            outcome.no_progress = false;
        }
    }
    if outcome.stop_signal.is_none()
        && prepared_round
            .effective_output_contract
            .as_ref()
            .is_some_and(|contract| {
                observe_only_round_should_continue(contract, loop_state, &actions)
            })
    {
        loop_state.has_recoverable_failure_context = true;
        loop_state.output_vars.insert(
            "agent_loop.observe_only_continue".to_string(),
            "true".to_string(),
        );
        outcome.stop_signal = Some("recoverable_failure_continue_round".to_string());
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
            .and_then(|ctx| ctx.output_contract())
            .is_some(),
        agent_run_context
            .and_then(|ctx| ctx.user_request.as_deref())
            .is_some_and(|text| !text.trim().is_empty())
    );
    crate::execution_recipe::ExecutionRecipeSpec::default()
}

pub(super) async fn run_agent_with_loop_with_initial_observations(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        None,
        None,
        initial_task_observations,
    )
    .await
}

pub(super) async fn run_agent_with_loop_seeded(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        resume_checkpoint,
        None,
        initial_task_observations,
    )
    .await
}

pub(super) async fn run_agent_with_loop_seeded_direct_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: &crate::task_lifecycle::TaskCheckpoint,
    initial_plan: &crate::PlanResult,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        Some(resume_checkpoint),
        Some(initial_plan),
        initial_task_observations,
    )
    .await
}

pub(super) async fn run_agent_with_loop_direct_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    initial_plan: &crate::PlanResult,
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        None,
        Some(initial_plan),
        &[],
    )
    .await
}

async fn run_agent_with_loop_seeded_and_initial_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
    initial_plan: Option<&crate::PlanResult>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    let base_policy = load_agent_loop_guard_policy(state);
    let mut task_budget_policy =
        crate::task_budget_contract::load_task_budget_policy(&state.skill_rt.workspace_root);
    clamp_child_task_budget_policy(task, &mut task_budget_policy);
    let mut loop_state = LoopState::new();
    super::seed_loop_state_for_agent_run(&mut loop_state, agent_run_context, resume_checkpoint);
    loop_state
        .task_observations
        .extend(initial_task_observations.iter().cloned());
    record_session_start_hooks(state, task, user_text, &mut loop_state).await;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        initial_execution_recipe_spec(goal, user_text, agent_run_context),
    );
    let budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, None);
    let mut policy = base_policy.adjusted_for_context(loop_state.execution_recipe, None);
    clamp_child_loop_guard_policy(task, &mut policy);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    let enabled_rollout_switches = policy.enabled_rollout_switches();
    if !enabled_rollout_switches.is_empty() {
        loop_state.output_vars.insert(
            "rollout_switches_enabled".to_string(),
            enabled_rollout_switches.join(","),
        );
    }
    info!(
        "loop_budget_profile task_id={} profile={} max_actions_per_turn={} repeat_action_limit={}",
        task.task_id,
        budget_profile.as_str(),
        policy.max_actions_per_turn,
        policy.repeat_action_limit
    );
    let mut round = 1usize;
    let loop_started_at = Instant::now();
    let worker_task_timeout_seconds = crate::worker::task_budget::task_execution_timeout_seconds(
        state.worker.worker_task_timeout_seconds,
        &task.kind,
        &task.payload_json,
    );
    let worker_soft_checkpoint_after = worker_soft_checkpoint_after(worker_task_timeout_seconds);
    initialize_task_budget_slice(
        &mut loop_state,
        budget_profile,
        &task_budget_policy,
        worker_soft_checkpoint_after,
        worker_task_timeout_seconds,
    );
    let mut skip_planner_rounds = false;
    loop {
        if !skip_planner_rounds {
            loop {
                ensure_task_running(state, task)?;
                loop_state.round_no = round;
                if task_budget_soft_slice_exhausted(loop_started_at, &loop_state) {
                    let decision = observe_task_budget(
                        state,
                        task,
                        &mut loop_state,
                        None,
                        loop_started_at,
                        true,
                    );
                    loop_state.last_stop_signal = Some("task_budget_slice_exhausted".to_string());
                    if matches!(
                        decision,
                        crate::task_budget_contract::BudgetDecision::CheckpointRequeue
                    ) {
                        publish_agent_loop_checkpoint_progress(
                            state,
                            task,
                            &mut loop_state,
                            "task_budget_slice_exhausted",
                        );
                    }
                    break;
                }
                super::maybe_publish_execution_recipe_phase_hint(state, task, &mut loop_state);
                let outcome = run_agent_round(
                    state,
                    task,
                    goal,
                    user_text,
                    &mut policy,
                    &task_budget_policy,
                    &mut loop_state,
                    agent_run_context,
                    (round == 1).then_some(initial_plan).flatten(),
                )
                .await?;
                loop_state.last_stop_signal = outcome.stop_signal.clone();
                if outcome.no_progress {
                    loop_state.consecutive_no_progress =
                        loop_state.consecutive_no_progress.saturating_add(1);
                } else {
                    loop_state.consecutive_no_progress = 0;
                }
                let soft_slice_exhausted =
                    task_budget_soft_slice_exhausted(loop_started_at, &loop_state);
                let decision = observe_task_budget(
                    state,
                    task,
                    &mut loop_state,
                    Some(&outcome),
                    loop_started_at,
                    soft_slice_exhausted,
                );
                match decision {
                    crate::task_budget_contract::BudgetDecision::Continue => {
                        round = round.saturating_add(1);
                    }
                    crate::task_budget_contract::BudgetDecision::CheckpointRequeue => {
                        loop_state.last_stop_signal =
                            Some("task_budget_slice_exhausted".to_string());
                        publish_agent_loop_checkpoint_progress(
                            state,
                            task,
                            &mut loop_state,
                            "task_budget_slice_exhausted",
                        );
                        break;
                    }
                    crate::task_budget_contract::BudgetDecision::Waiting => {
                        if let Some(resume_reason) =
                            recoverable_provider_blocker_resume_reason(&loop_state)
                        {
                            publish_agent_loop_checkpoint_progress(
                                state,
                                task,
                                &mut loop_state,
                                resume_reason,
                            );
                        }
                        break;
                    }
                    crate::task_budget_contract::BudgetDecision::NeedsUser
                    | crate::task_budget_contract::BudgetDecision::Finish
                    | crate::task_budget_contract::BudgetDecision::Terminal => break,
                }
            }
        }
        if loop_state_has_checkpoint_handoff(&loop_state) {
            return Ok(checkpoint_handoff_reply(
                task,
                user_text,
                &loop_state,
                agent_run_context,
            ));
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
        if loop_state_has_checkpoint_handoff(&pre_finalize_loop_state) {
            return Ok(reply);
        }
        let answer_contract = answer_contract_for_reply(user_text, &reply);
        prefer_terminal_model_answer_for_verifier_candidate(&mut reply, answer_contract.as_ref());
        enforce_post_write_content_evidence_guard(&mut reply);
        enforce_code_mutation_validation_success_guard(&mut reply);
        let mut pre_verifier_recovery_loop_state = pre_finalize_loop_state.clone();
        if try_run_post_write_validation_reserve_recovery(
            state,
            task,
            goal,
            user_text,
            &policy,
            &mut pre_verifier_recovery_loop_state,
            &reply,
            agent_run_context,
        )
        .await?
        {
            loop_state = pre_verifier_recovery_loop_state;
            skip_planner_rounds = true;
            continue;
        }
        attach_answer_verifier_if_missing(
            state,
            task,
            user_text,
            answer_contract.as_ref(),
            &mut reply,
        )
        .await;
        enforce_post_write_content_evidence_guard(&mut reply);
        enforce_code_mutation_validation_success_guard(&mut reply);
        let route_result = answer_contract.as_ref();
        suppress_answer_verifier_retry_if_structurally_satisfied(&mut reply, route_result);
        if let Some(verifier) = answer_verifier_retry_summary(&reply, route_result).cloned() {
            let mut verifier_replan_loop_state = pre_finalize_loop_state.clone();
            if prepare_answer_verifier_evidence_replan(&mut verifier_replan_loop_state, &verifier) {
                info!(
                    task_id = %task.task_id,
                    missing_evidence_fields = ?verifier.missing_evidence_fields,
                    "answer_verifier_evidence_replan"
                );
                loop_state = verifier_replan_loop_state;
                round = round.saturating_add(1);
                skip_planner_rounds = false;
                continue;
            }
            if let Some(route) = route_result {
                if try_bounded_answer_verifier_synthesis_retry(
                    state, task, user_text, route, &verifier, &mut reply,
                )
                .await
                {
                    info!("answer_verifier_bounded_synthesis_retry_succeeded");
                    return Ok(reply);
                }
            }
            warn!(
                task_id = %task.task_id,
                missing_evidence_fields = ?verifier.missing_evidence_fields,
                "answer_verifier_bounded_synthesis_retry_exhausted"
            );
            mark_reply_failed_after_answer_verifier_exhausted(user_text, &mut reply, &verifier);
        }
        return Ok(reply);
    }
}

#[cfg(test)]
#[path = "loop_control_tests.rs"]
mod tests;
