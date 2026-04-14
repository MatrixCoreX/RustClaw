use tracing::info;

use super::{
    ensure_task_running, execute_actions_once, finalize_loop_reply, load_agent_loop_guard_policy,
    prepare_round_actions, push_round_trace, AgentLoopGuardPolicy, AgentRunContext, LoopState,
    RoundOutcome,
};
use crate::{AgentAction, AppState, AskReply, ClaimedTask, RouteResult};

fn has_authoritative_delivery(loop_state: &LoopState) -> bool {
    !loop_state.delivery_messages.is_empty()
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
        || loop_state
            .last_publishable_chat_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
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

fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } => true,
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. } => {
            skill.eq_ignore_ascii_case("chat")
        }
        AgentAction::Think { .. } => false,
    })
}

fn has_executable_observation_or_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn should_stop_for_observed_finalize(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
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
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        if super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return true;
        }
        if super::observed_output::route_prefers_direct_observed_answer_for_scalar(route_result)
            && super::observed_output::extract_direct_answer_from_generic_output(
                loop_state,
                agent_run_context,
            )
            .is_some()
        {
            return true;
        }
    }
    has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && super::observed_output::has_observed_answer_candidates(loop_state)
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
    let actions = prepared_round.actions;
    let mut outcome =
        execute_actions_once(state, task, goal, user_text, &actions, loop_state, policy).await?;
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

pub(super) async fn run_agent_with_loop(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    let policy = load_agent_loop_guard_policy(state);
    let mut loop_state = LoopState::new(policy.max_rounds.max(1));
    super::seed_loop_state_from_agent_context(&mut loop_state, agent_run_context);
    for round in 1..=loop_state.max_rounds {
        ensure_task_running(state, task)?;
        loop_state.round_no = round;
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
        if evaluate_round_outcome(task, &mut loop_state, &policy, &outcome) {
            break;
        }
    }
    finalize_loop_reply(state, task, user_text, loop_state, agent_run_context).await
}

#[cfg(test)]
mod tests {
    use super::should_stop_for_observed_finalize;
    use crate::{
        agent_engine::{AgentRunContext, LoopState},
        executor::{StepExecutionResult, StepExecutionStatus},
        AgentAction, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
        OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
        RoutedMode, ScheduleKind,
    };
    use serde_json::json;

    fn route_result(shape: OutputResponseShape) -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::ChatAct,
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
                response_shape: shape,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }
    }

    fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    #[test]
    fn observed_scalar_output_can_stop_loop_without_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"extract_field"}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Scalar)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn observation_only_freeform_round_can_stop_for_observed_fallback() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "README.md\ndocs/\ncrates/\n",
        ));
        let actions = vec![AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path":"."}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Free)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn hidden_entries_scalar_output_can_stop_before_chat_followup() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let mut route = route_result(OutputResponseShape::Scalar);
        route.resolved_intent =
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
        route.output_contract.locator_hint = ".".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path":"."}),
            },
            AgentAction::CallSkill {
                skill: "chat".to_string(),
                args: json!({"text":"summarize hidden entries"}),
            },
        ];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }
}
