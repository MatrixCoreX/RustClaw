use super::*;
use crate::agent_engine::subagent_runtime::{
    SUBAGENT_STOP_SIGNAL_CAPABILITY_POLICY_REJECTED, SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED,
};

#[test]
fn policy_rejection_replans_without_retrying_scheduler_or_completed_child_failures() {
    assert_eq!(
        normalize_subagent_stop_signal(Some(
            SUBAGENT_STOP_SIGNAL_CAPABILITY_POLICY_REJECTED.into()
        )),
        (Some("recoverable_failure_continue_round".into()), true)
    );
    for signal in [
        "subagent_child_task_schedule_failed",
        SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED,
    ] {
        assert_eq!(
            normalize_subagent_stop_signal(Some(signal.into())),
            (Some(signal.into()), false)
        );
    }
    assert_eq!(normalize_subagent_stop_signal(None), (None, false));
}

#[test]
fn rejected_subagent_attempt_is_kept_for_parent_replanning() {
    let task = super::super::tests::test_task();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.task_observations.push(json!({
        "owner_layer": "subagent_runtime",
        "status": "rejected",
        "error_code": "child_task_capability_policy_incompatible",
        "global_step": 1,
        "step_in_round": 1,
    }));
    record_subagent_step_execution(
        &task,
        &mut loop_state,
        1,
        1,
        &json!({"allowed_capabilities": ["optional_probe.write"]}),
        "call_capability",
        Some(SUBAGENT_STOP_SIGNAL_CAPABILITY_POLICY_REJECTED),
        "test-fingerprint",
    )
    .unwrap();
    assert!(loop_state.has_recoverable_failure_context);
    assert_eq!(loop_state.attempt_ledger_entries.len(), 1);
    assert_eq!(
        loop_state.executed_step_results[0].status,
        crate::executor::StepExecutionStatus::Error
    );
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.successful_action_fingerprints.is_empty());
}
