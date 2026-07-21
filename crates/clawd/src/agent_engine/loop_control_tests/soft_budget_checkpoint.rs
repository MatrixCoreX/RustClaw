use super::super::{
    loop_state_has_checkpoint_handoff, loop_state_has_recoverable_checkpoint_state,
    recoverable_provider_blocker_resume_reason, worker_soft_checkpoint_after_seconds,
};
use super::LoopState;

#[test]
fn provider_blocker_uses_machine_wait_reason() {
    let mut provider_blocker_state = LoopState::new();
    provider_blocker_state.round_no = 1;
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut provider_blocker_state,
        "image_generate",
        "action=generate",
        crate::executor::StepExecutionStatus::Error,
        "",
        None,
        &crate::skills::structured_skill_error_from_parts(
            "image_generate",
            "provider_retryable_response",
            "provider retryable response",
            None,
            Some(serde_json::json!({
                "provider": "minimax",
                "provider_error_class": "rate_limited",
                "external_provider_blocked": true,
                "retry_after_seconds": 60
            })),
        ),
    );
    assert_eq!(
        recoverable_provider_blocker_resume_reason(&provider_blocker_state),
        Some("provider_blocker_wait_background")
    );
}

#[test]
fn worker_soft_checkpoint_deadline_keeps_hard_timeout_reserve() {
    assert_eq!(worker_soft_checkpoint_after_seconds(1), None);
    assert_eq!(worker_soft_checkpoint_after_seconds(2), None);
    assert_eq!(worker_soft_checkpoint_after_seconds(3), Some(2));
    assert_eq!(worker_soft_checkpoint_after_seconds(10), Some(9));
    assert_eq!(worker_soft_checkpoint_after_seconds(3600), Some(3570));
}

#[test]
fn checkpoint_handoff_requires_matching_nonterminal_machine_state() {
    let mut loop_state = LoopState::new();
    loop_state.task_lifecycle = Some(serde_json::json!({
        "state": "waiting",
        "checkpoint_id": "checkpoint-1"
    }));
    loop_state.task_checkpoint = Some(serde_json::json!({
        "checkpoint_id": "checkpoint-1",
        "resume_entrypoint": "next_planner_round"
    }));
    assert!(loop_state_has_checkpoint_handoff(&loop_state));

    loop_state.task_lifecycle.as_mut().expect("lifecycle")["state"] = serde_json::json!("running");
    assert!(!loop_state_has_checkpoint_handoff(&loop_state));

    loop_state.task_lifecycle.as_mut().expect("lifecycle")["state"] =
        serde_json::json!("background");
    loop_state.task_checkpoint.as_mut().expect("checkpoint")["checkpoint_id"] =
        serde_json::json!("checkpoint-other");
    assert!(!loop_state_has_checkpoint_handoff(&loop_state));
}

#[test]
fn recoverable_checkpoint_state_requires_machine_progress() {
    let empty = LoopState::new();
    assert!(!loop_state_has_recoverable_checkpoint_state(&empty));

    let mut with_step = LoopState::new();
    with_step
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("{\"status\":\"ok\"}".to_string()),
            error: None,
            started_at: 10,
            finished_at: 11,
        });
    assert!(loop_state_has_recoverable_checkpoint_state(&with_step));
}
