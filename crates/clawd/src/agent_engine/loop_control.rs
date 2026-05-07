use tracing::{info, warn};

use super::{
    ensure_task_running, execute_actions_once, load_agent_loop_guard_policy, prepare_round_actions,
    push_round_trace, AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
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
            .last_publishable_synthesis_output
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
        AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::Think { .. } => false,
        AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => false,
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

pub(crate) fn requested_success_marker(
    agent_run_context: Option<&AgentRunContext>,
) -> Option<&'static str> {
    let request = agent_run_context
        .and_then(|ctx| ctx.user_request.as_deref())
        .or_else(|| {
            agent_run_context
                .and_then(|ctx| ctx.route_result.as_ref())
                .map(|route| route.resolved_intent.as_str())
        })?;
    let upper = request.to_ascii_uppercase();
    if upper.contains("VALIDATION_PASSED") {
        Some("VALIDATION_PASSED")
    } else {
        None
    }
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
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && has_direct_observed_answer
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
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
        && super::observed_output::has_observed_answer_candidates(loop_state);
    can_stop
        && required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        })
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
        agent_run_context.and_then(|ctx| ctx.route_result.as_ref()).is_some(),
        agent_run_context
            .and_then(|ctx| ctx.user_request.as_deref())
            .is_some_and(|text| !text.trim().is_empty())
    );
    crate::execution_recipe::ExecutionRecipeSpec::default()
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
    let policy = base_policy.adjusted_for_recipe(loop_state.execution_recipe);
    loop_state.max_rounds = policy.max_rounds.max(1);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    for round in 1..=loop_state.max_rounds {
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
    }
    crate::finalize::finalize_loop_reply(state, task, user_text, loop_state, agent_run_context)
        .await
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_round_outcome, initial_execution_recipe_spec, should_stop_for_observed_finalize,
        AgentLoopGuardPolicy, RoundOutcome,
    };
    use crate::{
        agent_engine::{AgentRunContext, LoopState},
        execution_recipe::{
            ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
            ExecutionRecipeSpec, ExecutionRecipeTargetScope,
        },
        executor::{StepExecutionResult, StepExecutionStatus},
        AgentAction, ClaimedTask, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
        OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult,
        RoutedMode, ScheduleKind,
    };
    use serde_json::json;

    fn route_result(shape: OutputResponseShape) -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(RoutedMode::ChatAct),
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

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "task-loop-control".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn test_policy() -> AgentLoopGuardPolicy {
        AgentLoopGuardPolicy {
            max_steps: 8,
            max_rounds: 4,
            repeat_action_limit: 3,
            no_progress_limit: 1,
            multi_round_enabled: true,
            ops_closed_loop: Default::default(),
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
    fn unscoped_workspace_evidence_drafting_does_not_stop_on_search_only() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":2,"results":["README.md","USAGE.md"]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "Write a short setup note grounded in the current workspace docs".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({"action":"find_name","pattern":"README"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn unscoped_workspace_evidence_drafting_can_stop_after_doc_read() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|## Setup"}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "Write a short setup note grounded in the current workspace docs".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"read_range","path":"README.md","mode":"head","n":120}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn hidden_entries_scalar_output_can_stop_before_synthesis_followup() {
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
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
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

    #[test]
    fn existence_with_path_free_output_can_stop_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":1190},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "rustclaw.service".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"path_batch_facts","paths":["/home/guagua/rustclaw/rustclaw.service"]}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn structured_keys_free_output_can_stop_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.route_reason = "llm_contract:generic_explicit_path_structured_keys".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/package.json".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"structured_keys","path":"/tmp/package.json","field_path":"scripts"}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn extract_fields_free_output_can_stop_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.route_reason = "llm_contract:generic_explicit_path_extract_fields".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/config.toml".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"extract_fields","path":"/tmp/config.toml","field_paths":["database.sqlite_path","tools.allow_sudo"]}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn health_check_scalar_summary_continues_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        let mut route = route_result(OutputResponseShape::Scalar);
        route.resolved_intent =
            "执行基础健康检查，仅提取并返回操作系统相关的关键字段，排除 RustClaw 自身的状态摘要"
                .to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: json!({}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recipe_waiting_for_validation_does_not_stop_on_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            validation_required: true,
            saw_mutation: true,
            ..Default::default()
        };
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "configuration updated\n",
        ));
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command":"cat ./config.json"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Free)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recipe_inspect_stage_does_not_stop_on_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Inspect,
            inspect_first: true,
            validation_required: true,
            ..Default::default()
        };
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "index.html\n"));
        let actions = vec![AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path":"document/nl_ops_http_demo"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Scalar)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recipe_done_still_waits_for_requested_success_marker() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "ops-demo-ok\n"));
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command":"curl -s http://127.0.0.1:52752/ | grep -o ops-demo-ok"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Scalar)),
                user_request: Some(
                    "验证通过时请明确输出 VALIDATION_PASSED，然后直接结束。".to_string()
                ),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recoverable_recipe_failure_continues_next_round_and_keeps_repair_count() {
        let task = test_task();
        let policy = test_policy();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
            inspect_first: true,
            validation_required: true,
            max_repairs: 3,
            repair_count: 1,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            next_goal_hint: Some("repair sing-box".to_string()),
            no_progress: false,
        };
        assert!(!evaluate_round_outcome(
            &task,
            &mut loop_state,
            &policy,
            &outcome
        ));
        assert_eq!(loop_state.execution_recipe.repair_count, 1);
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
        assert_eq!(loop_state.consecutive_no_progress, 0);
    }

    #[test]
    fn exhausted_recipe_budget_stops_next_round() {
        let task = test_task();
        let policy = test_policy();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 2;
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            repair_count: 3,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some("recipe_repair_budget_exhausted".to_string()),
            next_goal_hint: None,
            no_progress: false,
        };
        assert!(evaluate_round_outcome(
            &task,
            &mut loop_state,
            &policy,
            &outcome
        ));
        assert_eq!(loop_state.execution_recipe.repair_count, 3);
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
    }

    #[test]
    fn explicit_execution_recipe_hint_takes_priority_over_local_detection() {
        let spec = initial_execution_recipe_spec(
            "configure sing-box and verify the proxy works",
            "configure sing-box and verify the proxy works",
            Some(&AgentRunContext {
                execution_recipe_hint: Some(ExecutionRecipeSpec {
                    kind: ExecutionRecipeKind::OpsClosedLoop,
                    profile: ExecutionRecipeProfile::CodeChange,
                    target_scope: ExecutionRecipeTargetScope::Greenfield,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                }),
                route_result: Some(route_result(OutputResponseShape::Free)),
                user_request: Some("configure sing-box and verify the proxy works".to_string()),
                ..Default::default()
            }),
        );
        assert_eq!(spec.profile, ExecutionRecipeProfile::CodeChange);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::Greenfield);
    }
}
