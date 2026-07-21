use super::{
    answer_contract_for_reply, answer_verifier_retry_summary,
    apply_structured_respond_clarify_to_loop_state, commit_answer_verifier_retry_answer,
    forced_boundary_observation_clarify_intent, initial_execution_recipe_spec,
    observe_only_round_should_continue, post_write_content_evidence_recovery_policy,
    prefer_terminal_model_answer_for_verifier_candidate,
    promote_local_code_projection_from_machine_evidence_for_verifier_candidate,
    promote_publishable_strict_json_projection_for_verifier_candidate,
    record_agent_loop_decision_envelope_output_vars, retry_verifier_accepts_rewritten_answer,
    round_model_finished, should_stop_for_observed_finalize,
    structured_field_selector_observation_can_finalize,
    structured_respond_terminal_intent_from_plan,
    suppress_answer_verifier_retry_if_structurally_satisfied, terminal_user_answer_stop_signal,
    text_has_exact_marker_line, try_recover_inconsistent_boundary_clarify, AgentLoopGuardPolicy,
    RoundOutcome,
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
fn success_marker_matching_requires_exact_line() {
    assert!(!text_has_exact_marker_line(
        "status=ok\nVALIDATION_PASSED_EXTRA",
        "VALIDATION_PASSED",
    ));
    assert!(text_has_exact_marker_line(
        "status=ok\nVALIDATION_PASSED\nnext=done",
        "VALIDATION_PASSED",
    ));
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
        max_steps: 8,
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
