use tracing::info;

use super::{AgentLoopGuardPolicy, AgentRunContext, LoopState};
use crate::{AgentAction, AppState, ClaimedTask, PlanResult};

pub(super) struct PreparedRoundActions {
    pub(super) actions: Vec<AgentAction>,
    pub(super) plan_result: PlanResult,
    pub(super) verify_result: crate::verifier::VerifyResult,
}

fn build_round_verify_summary(
    verify_result: &crate::verifier::VerifyResult,
) -> crate::task_journal::TaskJournalVerifySummary {
    crate::task_journal::TaskJournalVerifySummary {
        mode: verify_result.mode,
        approved: verify_result.approved,
        blocked_reason: verify_result.blocked_reason.clone(),
        shadow_blocked_reason: verify_result.shadow_blocked_reason.clone(),
        needs_confirmation: verify_result.needs_confirmation,
        issues: verify_result
            .issues
            .iter()
            .map(|issue| crate::task_journal::TaskJournalVerifyIssue {
                step_id: issue.step_id.clone(),
                kind: issue.kind,
                detail: issue.detail.clone(),
            })
            .collect(),
    }
}

fn verify_mode_for_state(state: &AppState) -> crate::verifier::VerifyMode {
    if state.command_intent.verify_enforce_enabled {
        crate::verifier::VerifyMode::Enforce
    } else {
        crate::verifier::VerifyMode::ObserveOnly
    }
}

fn build_verifier_gate_response(
    state: &AppState,
    verify_result: &crate::verifier::VerifyResult,
) -> String {
    let prefer_english = state
        .command_intent
        .default_locale
        .to_ascii_lowercase()
        .starts_with("en");
    let first_detail = verify_result
        .issues
        .first()
        .map(|issue| crate::truncate_for_log(&issue.detail))
        .or_else(|| verify_result.blocked_reason.clone())
        .unwrap_or_else(|| "plan failed verification".to_string());
    let needs_confirmation = verify_result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            crate::verifier::VerifyIssueKind::ConfirmationRequired
        )
    });
    let needs_clarify = verify_result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            crate::verifier::VerifyIssueKind::RouteClarifyRequired
        )
    });
    if needs_confirmation {
        crate::bilingual_t_with_default_vars(
            state,
            "clawd.msg.verify_gate_confirmation_required",
            "这一步需要你先明确确认，我还不会直接执行。\n原因：{detail}",
            "This step needs your explicit confirmation before I execute it.\nReason: {detail}",
            prefer_english,
            &[("detail", &first_detail)],
        )
    } else if needs_clarify {
        crate::bilingual_t_with_default_vars(
            state,
            "clawd.msg.verify_gate_clarify_required",
            "你的需求还需要先补充澄清，我先不执行。\n原因：{detail}",
            "I need a clarification before executing this plan.\nReason: {detail}",
            prefer_english,
            &[("detail", &first_detail)],
        )
    } else {
        crate::bilingual_t_with_default_vars(
            state,
            "clawd.msg.verify_gate_blocked",
            "当前计划未通过执行前校验，已停止执行。\n原因：{detail}",
            "The current plan did not pass pre-execution verification, so execution was stopped.\nReason: {detail}",
            prefer_english,
            &[("detail", &first_detail)],
        )
    }
}

pub(super) async fn prepare_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<PreparedRoundActions, String> {
    let plan_result = super::planning::plan_round_actions(
        state,
        task,
        goal,
        user_text,
        policy,
        loop_state,
        agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
    )
    .await?;
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
    let verify_mode = verify_mode_for_state(state);
    let verify_result = crate::verifier::verify_plan(
        state,
        task,
        crate::verifier::VerifyInput {
            route_result: agent_run_context.and_then(|ctx| ctx.route_result.as_ref()),
            context_bundle_summary: agent_run_context
                .and_then(|ctx| ctx.context_bundle_summary.as_deref()),
            plan_result: &plan_result,
        },
        verify_mode,
    );
    info!(
        "verifier_result task_id={} round={} mode={:?} approved={} needs_confirmation={} issue_count={} blocked_reason={} shadow_blocked_reason={}",
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
    let mut journal = crate::task_journal::TaskJournal::new(user_text);
    if let Some(route_result) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        journal.record_route_result(route_result);
    }
    journal.record_plan_result(&plan_result);
    journal.record_verify_result(&verify_result);
    let context_summary = agent_run_context
        .and_then(|ctx| ctx.context_bundle_summary.as_deref())
        .unwrap_or("<none>");
    journal.record_context_bundle_summary(format!(
        "round={} goal={} context={}",
        loop_state.round_no,
        crate::truncate_for_log(goal),
        context_summary
    ));
    info!(
        "task_journal_summary task_id={} kind=ask phase=plan_verify round={} {}",
        task.task_id,
        loop_state.round_no,
        journal.to_log_json()
    );
    let actions = if matches!(verify_result.mode, crate::verifier::VerifyMode::Enforce)
        && (!verify_result.approved || verify_result.needs_confirmation)
    {
        vec![AgentAction::Respond {
            content: build_verifier_gate_response(state, &verify_result),
        }]
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
        plan_result.to_agent_actions()
    };
    Ok(PreparedRoundActions {
        actions,
        plan_result,
        verify_result,
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
            plan_result: Some(prepared_round.plan_result.clone()),
            verify_result: Some(build_round_verify_summary(&prepared_round.verify_result)),
        });
}
