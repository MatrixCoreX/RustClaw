use tracing::info;

use super::{AgentLoopGuardPolicy, AgentRunContext, LoopState};
use crate::{AgentAction, AppState, ClaimedTask, PlanResult};

pub(super) struct PreparedRoundActions {
    pub(super) actions: Vec<AgentAction>,
    pub(super) plan_result: PlanResult,
    pub(super) verify_result: crate::verifier::VerifyResult,
    pub(super) effective_output_contract: Option<crate::IntentOutputContract>,
}

fn build_round_verify_summary(
    verify_result: &crate::verifier::VerifyResult,
) -> crate::task_journal::TaskJournalVerifySummary {
    crate::task_journal::TaskJournalVerifySummary {
        mode: verify_result.mode,
        approved: verify_result.approved,
        blocked_reason: verify_result.blocked_reason.clone(),
        shadow_blocked_reason: verify_result.shadow_blocked_reason.clone(),
        permission_decision: verify_result.permission_decision.clone(),
        needs_confirmation: verify_result.needs_confirmation,
        issues: verify_result
            .issues
            .iter()
            .map(|issue| crate::task_journal::TaskJournalVerifyIssue {
                step_id: issue.step_id.clone(),
                kind: issue.kind,
                detail: issue.detail.clone(),
                missing_fields: issue.missing_fields.clone(),
            })
            .collect(),
    }
}

fn production_verify_mode() -> crate::verifier::VerifyMode {
    crate::verifier::VerifyMode::Enforce
}

async fn build_verifier_gate_response(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resolved_user_intent: &str,
    verify_result: &crate::verifier::VerifyResult,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let first_issue_kind = verify_result
        .issues
        .first()
        .map(|issue| issue.kind.as_str())
        .unwrap_or("none");
    let needs_confirmation = verifier_gate_needs_confirmation(verify_result);
    let needs_clarify = verifier_gate_needs_clarification(verify_result);
    let (reason_code, missing_slots, response_shape, fallback_source) = if needs_confirmation {
        (
            "execution_confirmation_required",
            vec!["explicit_user_confirmation".to_string()],
            "one_short_confirmation_question",
            crate::fallback::ClarifyFallbackSource::VerifyRejected,
        )
    } else if needs_clarify {
        (
            "execution_clarification_required",
            verifier_gate_missing_slots(verify_result),
            "one_short_clarification",
            crate::fallback::ClarifyFallbackSource::VerifyRejected,
        )
    } else {
        (
            "execution_precheck_blocked",
            Vec::new(),
            "brief_failure_with_next_step",
            crate::fallback::ClarifyFallbackSource::PolicyBlock,
        )
    };
    let mut observed_facts = vec![
        format!("verification_issue_kind: {first_issue_kind}"),
        format!("verification_issue_count: {}", verify_result.issues.len()),
        format!("needs_confirmation: {needs_confirmation}"),
        format!("needs_clarification: {needs_clarify}"),
    ];
    if verify_result
        .blocked_reason
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        observed_facts.push("blocked_reason_present: true".to_string());
    }
    let contract = crate::fallback::UserResponseContract::verifier_gate(
        reason_code,
        user_text,
        resolved_user_intent,
        missing_slots,
        observed_facts,
        response_shape,
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract(state, task, &contract, fallback_source)
        .await
}

fn verifier_gate_needs_confirmation(verify_result: &crate::verifier::VerifyResult) -> bool {
    verify_result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            crate::verifier::VerifyIssueKind::ConfirmationRequired
        )
    })
}

fn verifier_gate_needs_clarification(verify_result: &crate::verifier::VerifyResult) -> bool {
    verify_result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            crate::verifier::VerifyIssueKind::BoundaryClarifyRequired
                | crate::verifier::VerifyIssueKind::MissingRequiredArg
        )
    })
}

fn verifier_gate_missing_slots(verify_result: &crate::verifier::VerifyResult) -> Vec<String> {
    let mut slots = Vec::new();
    for issue in &verify_result.issues {
        let slot = match issue.kind {
            crate::verifier::VerifyIssueKind::BoundaryClarifyRequired => {
                "execution_target_or_boundary"
            }
            crate::verifier::VerifyIssueKind::MissingRequiredArg => "required_execution_argument",
            _ => continue,
        };
        if !slots.iter().any(|existing| existing == slot) {
            slots.push(slot.to_string());
        }
    }
    if slots.is_empty() {
        slots.push("execution_target_or_boundary".to_string());
    }
    slots
}

fn verifier_gate_should_stop_round(verify_result: &crate::verifier::VerifyResult) -> bool {
    if matches!(verify_result.mode, crate::verifier::VerifyMode::Enforce)
        && (!verify_result.approved || verify_result.needs_confirmation)
    {
        return true;
    }
    verifier_gate_needs_clarification(verify_result)
}

fn planner_user_text<'a>(
    agent_run_context: Option<&'a AgentRunContext>,
    fallback: &'a str,
) -> &'a str {
    agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .filter(|text| !text.trim().is_empty())
        .or_else(|| agent_run_context.and_then(|ctx| ctx.user_request.as_deref()))
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(fallback)
}

pub(super) async fn prepare_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    initial_plan: Option<&crate::PlanResult>,
) -> Result<PreparedRoundActions, String> {
    let planner_user_text = planner_user_text(agent_run_context, user_text);
    let effective_goal = loop_state
        .execution_recipe
        .goal_overlay()
        .map(|overlay| format!("{goal}\n\n{overlay}"))
        .unwrap_or_else(|| goal.to_string());
    let plan_result = if let Some(initial_plan) = initial_plan {
        initial_plan.clone()
    } else {
        super::planning::plan_round_actions(
            state,
            task,
            &effective_goal,
            planner_user_text,
            policy,
            loop_state,
            agent_run_context.and_then(|ctx| ctx.turn_analysis.as_ref()),
            agent_run_context.and_then(|ctx| ctx.boundary_envelope.as_ref()),
            agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref()),
        )
        .await?
    };
    info!(
        "planner_result task_id={} round={} plan_kind={:?} goal={} step_count={} missing_slots={} needs_confirmation={} planner_notes={} raw_plan={}",
        task.task_id,
        loop_state.round_no,
        plan_result.plan_kind,
        crate::truncate_for_log(&plan_result.goal),
        plan_result.steps.len(),
        serde_json::to_string(&plan_result.missing_slots).unwrap_or_else(|_| "[]".to_string()),
        plan_result.needs_confirmation,
        crate::truncate_for_log(&plan_result.planner_notes),
        crate::truncate_for_log(&plan_result.raw_plan_text)
    );
    let effective_output_contract =
        crate::capability_resolver::bind_unclassified_output_contract_from_capabilities(
            state,
            &plan_result,
        );
    let verify_mode = production_verify_mode();
    let verify_result = crate::verifier::verify_plan(
        state,
        task,
        crate::verifier::VerifyInput {
            output_contract: effective_output_contract.as_ref(),
            request_text: Some(planner_user_text),
            context_bundle_summary: agent_run_context
                .and_then(|ctx| ctx.context_bundle_summary.as_deref()),
            plan_result: &plan_result,
            execution_recipe: loop_state.execution_recipe,
        },
        verify_mode,
    );
    info!(
        "verifier_result task_id={} round={} verifier_mode={:?} approved={} needs_confirmation={} issue_count={} blocked_reason={} shadow_blocked_reason={}",
        task.task_id,
        loop_state.round_no,
        verify_result.mode,
        verify_result.approved,
        verify_result.needs_confirmation,
        verify_result.issues.len(),
        crate::truncate_for_log(verify_result.blocked_reason.as_deref().unwrap_or("")),
        crate::truncate_for_log(verify_result.shadow_blocked_reason.as_deref().unwrap_or(""))
    );
    for issue in &verify_result.issues {
        info!(
            "verifier_issue task_id={} round={} step_id={} kind={:?} detail={}",
            task.task_id,
            loop_state.round_no,
            issue.step_id,
            issue.kind,
            crate::truncate_for_log(&issue.detail)
        );
    }
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_text);
    journal.record_plan_result(&plan_result);
    journal.record_verify_result(&verify_result);
    let context_summary = agent_run_context
        .and_then(|ctx| ctx.context_bundle_summary.as_deref())
        .unwrap_or("<none>");
    journal.record_context_bundle_summary(format!(
        "round={} goal={} context={} recipe={}",
        loop_state.round_no,
        crate::truncate_for_log(goal),
        context_summary,
        loop_state.execution_recipe.phase_summary_line()
    ));
    info!(
        "task_journal_summary task_id={} kind=ask phase=plan_verify round={} {}",
        task.task_id,
        loop_state.round_no,
        journal.to_log_json()
    );
    let actions = if verifier_gate_should_stop_round(&verify_result) {
        let content = build_verifier_gate_response(
            state,
            task,
            planner_user_text,
            &plan_result.goal,
            &verify_result,
        )
        .await;
        vec![AgentAction::Respond { content }]
    } else if matches!(verify_result.mode, crate::verifier::VerifyMode::Enforce) {
        let verified_steps = if !verify_result.rewritten_steps.is_empty() {
            &verify_result.rewritten_steps
        } else {
            &verify_result.approved_steps
        };
        verified_steps
            .iter()
            .filter_map(|step| step.to_agent_action())
            .collect()
    } else {
        let verified_steps = if !verify_result.rewritten_steps.is_empty() {
            &verify_result.rewritten_steps
        } else {
            &verify_result.approved_steps
        };
        verified_steps
            .iter()
            .filter_map(|step| step.to_agent_action())
            .collect()
    };
    Ok(PreparedRoundActions {
        actions,
        plan_result,
        verify_result,
        effective_output_contract,
    })
}

pub(super) fn push_round_trace(
    loop_state: &mut LoopState,
    goal: &str,
    prepared_round: &PreparedRoundActions,
) {
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: loop_state.round_no,
            goal: goal.to_string(),
            execution_recipe_summary: loop_state
                .execution_recipe
                .is_active()
                .then(|| loop_state.execution_recipe.phase_summary_line()),
            plan_result: Some(prepared_round.plan_result.clone()),
            verify_result: Some(build_round_verify_summary(&prepared_round.verify_result)),
        });
}

#[cfg(test)]
#[path = "prepare_round_tests.rs"]
mod tests;
