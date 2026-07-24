use super::{
    answer_contract_for_reply, answer_verifier_retry_summary,
    apply_structured_respond_clarify_to_loop_state, budget_replan_cause, child_loop_budget_limits,
    coding_workflow_ready_for_model_finalization, commit_answer_verifier_retry_answer,
    forced_boundary_observation_clarify_intent, initial_execution_recipe_spec,
    next_resumable_budget_action, observe_only_round_should_continue,
    post_write_content_evidence_recovery_policy,
    prefer_terminal_model_answer_for_verifier_candidate,
    record_agent_loop_decision_envelope_output_vars, retry_rewritten_answer_is_publishable,
    retry_verifier_accepts_rewritten_answer, round_is_policy_terminal, round_model_finished,
    select_round_task_budget_profile, should_stop_for_observed_finalize,
    structured_field_selector_observation_can_finalize,
    structured_respond_terminal_intent_from_plan,
    suppress_answer_verifier_retry_if_structurally_satisfied, terminal_user_answer_stop_signal,
    try_recover_inconsistent_boundary_clarify, AgentLoopGuardPolicy, RoundOutcome,
};
use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, RegistryIdempotencyGuardScope,
};
use crate::{
    agent_engine::{AgentRunContext, LoopState},
    execution_recipe::{
        ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
        ExecutionRecipeSpec, ExecutionRecipeTargetScope,
    },
    executor::{StepExecutionResult, StepExecutionStatus},
    AgentAction, AskReply, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape,
};
use serde_json::json;

#[test]
fn child_loop_budget_comes_only_from_structured_child_contract() {
    let limits = child_loop_budget_limits(
        &json!({
            "task_role": "subagent_child",
            "child_task_contract": {
                "budget": {
                    "max_rounds": 7,
                    "max_tool_calls": 11,
                    "timeout_ms": 180000
                }
            }
        })
        .to_string(),
    )
    .expect("child limits");
    assert_eq!(limits.max_rounds, 7);
    assert_eq!(limits.max_tool_calls, 11);
    assert_eq!(limits.timeout_ms, 180000);
    assert!(child_loop_budget_limits(
        &json!({
            "task_role": "ordinary",
            "child_task_contract": {"budget": {"max_rounds": 1}}
        })
        .to_string()
    )
    .is_none());
}

#[test]
fn zero_action_verifier_replan_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("replan_from_verifier_signal".to_string()),
        no_progress: false,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn zero_action_observation_ready_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("structured_observation_already_ready".to_string()),
        next_goal_hint: None,
        no_progress: true,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn capability_scope_load_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("capability_groups_loaded".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn mcp_capability_scope_load_round_is_not_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("mcp_capabilities_loaded".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };

    assert!(!round_model_finished(Some(&outcome)));
}

#[test]
fn budget_telemetry_uses_machine_replan_and_resume_tokens() {
    use crate::task_budget_contract::BudgetDecision;

    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("post_write_validation_reserve".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };
    assert_eq!(
        budget_replan_cause(BudgetDecision::Continue, Some(&outcome)),
        Some("post_write_validation_reserve")
    );
    assert_eq!(
        budget_replan_cause(BudgetDecision::Finish, Some(&outcome)),
        None
    );
    assert_eq!(
        next_resumable_budget_action(BudgetDecision::CheckpointRequeue, None),
        Some("resume_checkpoint")
    );
    assert_eq!(
        next_resumable_budget_action(BudgetDecision::Waiting, Some("background")),
        Some("poll_async_job")
    );
    assert_eq!(
        next_resumable_budget_action(BudgetDecision::NeedsUser, Some("needs_user")),
        Some("await_user_input")
    );
}

#[test]
fn verifier_retry_cannot_publish_unobserved_local_code_status() {
    assert!(!retry_rewritten_answer_is_publishable(
        r#"{"changed_files":["test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed_in_trace"}"#,
    ));
    assert!(!retry_rewritten_answer_is_publishable(
        r#"{"status":"not_observed_in_trace"}"#,
    ));
    assert!(retry_rewritten_answer_is_publishable(
        "The trace notes that no unresolved machine status was returned.",
    ));
}

#[test]
fn later_verified_plan_budget_can_widen_but_not_narrow() {
    use crate::task_budget_contract::TaskBudgetProfile;

    assert_eq!(
        select_round_task_budget_profile(None, TaskBudgetProfile::FastRead),
        (TaskBudgetProfile::FastRead, true)
    );
    assert_eq!(
        select_round_task_budget_profile(
            Some(TaskBudgetProfile::FastRead),
            TaskBudgetProfile::MultiStepWorkspace,
        ),
        (TaskBudgetProfile::MultiStepWorkspace, true)
    );
    assert_eq!(
        select_round_task_budget_profile(
            Some(TaskBudgetProfile::MultiStepWorkspace),
            TaskBudgetProfile::FastRead,
        ),
        (TaskBudgetProfile::MultiStepWorkspace, false)
    );
}

#[test]
fn verified_coding_workflow_hands_off_to_model_finalization() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"src/lib.rs","resolved_path":"/workspace/src/lib.rs"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "run_cmd",
        r#"{"extra":{"command":"cargo test -p demo"}}"#,
    ));

    assert!(coding_workflow_ready_for_model_finalization(&loop_state));
}

#[test]
fn unverified_or_read_only_workflow_does_not_finalize() {
    let mut unverified = LoopState::new();
    unverified.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"src/lib.rs","resolved_path":"/workspace/src/lib.rs"}}"#,
    ));
    assert!(!coding_workflow_ready_for_model_finalization(&unverified));

    let mut read_only = LoopState::new();
    read_only.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        r#"{"extra":{"command":"cargo test -p demo"}}"#,
    ));
    assert!(!coding_workflow_ready_for_model_finalization(&read_only));
}

#[test]
fn latest_command_validation_closes_workflow_when_step_output_omits_command() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"src/lib.rs","resolved_path":"/workspace/src/lib.rs"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "run_cmd",
        "test result without command metadata",
    ));
    loop_state.latest_validation_result = Some(serde_json::json!({
        "status": "passed",
        "verification_scope": "command",
        "global_step": 2,
    }));

    assert!(coding_workflow_ready_for_model_finalization(&loop_state));

    loop_state.latest_validation_result = Some(serde_json::json!({
        "status": "failed",
        "verification_scope": "command",
        "global_step": 2,
    }));
    assert!(!coding_workflow_ready_for_model_finalization(&loop_state));
}

#[test]
fn zero_action_terminal_round_is_model_finished() {
    let outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("respond".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };

    assert!(round_model_finished(Some(&outcome)));
}

#[test]
fn repeated_completed_action_replans_until_the_repeat_limit() {
    let replan = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("repeat_completed_action".to_string()),
        next_goal_hint: None,
        no_progress: true,
    };
    assert!(!round_is_policy_terminal(Some(&replan)));

    let exhausted = RoundOutcome {
        stop_signal: Some("repeat_action_limit".to_string()),
        ..replan
    };
    assert!(round_is_policy_terminal(Some(&exhausted)));
}

fn route_result(shape: OutputResponseShape) -> IntentOutputContract {
    IntentOutputContract {
        exact_sentence_count: None,
        response_shape: shape,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        locator_hint: String::new(),
        selection: crate::OutputSelectionContract::default(),
    }
}

fn answer_contract(route: &IntentOutputContract) -> crate::answer_verifier::AnswerContract {
    crate::answer_verifier::AnswerContract::new("test request", route.clone())
}

#[test]
fn answer_contract_for_reply_uses_journal_output_contract() {
    let mut output_contract = IntentOutputContract::default();
    output_contract.response_shape = OutputResponseShape::Strict;
    output_contract.selection.structured_field_selector = Some("path".to_string());
    output_contract.locator_kind = OutputLocatorKind::Path;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-effective-route",
        "ask",
        "probe service status",
    );
    journal.record_output_contract(&output_contract);
    let reply = AskReply::non_llm("ok".to_string()).with_task_journal(journal);

    let selected =
        answer_contract_for_reply("probe service status", &reply).expect("answer contract");

    assert_eq!(
        selected
            .output_contract
            .selection
            .structured_field_selector
            .as_deref(),
        Some("path")
    );
    assert_eq!(
        crate::evidence_policy::required_evidence_fields_for_output_contract(
            &selected.output_contract,
        ),
        vec!["path".to_string()]
    );
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

fn plan_result_with_raw_and_steps(
    raw_plan_text: &str,
    steps: Vec<crate::PlanStep>,
) -> crate::PlanResult {
    crate::PlanResult {
        goal: "test".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps,
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: raw_plan_text.to_string(),
    }
}

fn test_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_actions_per_turn: 8,
        repeat_action_limit: 3,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

#[path = "loop_control_tests/clarify_control.rs"]
mod clarify_control;
#[path = "loop_control_tests/observed_finalize.rs"]
mod observed_finalize;
#[path = "loop_control_tests/post_write_validation_reserve.rs"]
mod post_write_validation_reserve;
#[path = "loop_control_tests/soft_budget_checkpoint.rs"]
mod soft_budget_checkpoint;
#[path = "loop_control_tests/terminal_answer_stop.rs"]
mod terminal_answer_stop;
#[path = "loop_control_tests/verifier_retry_suppression.rs"]
mod verifier_retry_suppression;
