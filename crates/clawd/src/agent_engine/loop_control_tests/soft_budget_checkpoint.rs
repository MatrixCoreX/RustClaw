use super::super::{
    loop_state_has_recoverable_checkpoint_state, recoverable_provider_blocker_resume_reason,
    soft_budget_checkpoint_resume_reason, worker_soft_checkpoint_after_seconds,
};
use super::{test_policy, LoopState, RoundOutcome};

#[test]
fn soft_budget_checkpoint_reason_only_marks_budget_stops() {
    let policy = test_policy();
    let mut max_rounds_state = LoopState::new(2);
    max_rounds_state.round_no = 2;
    max_rounds_state.max_rounds = 2;
    max_rounds_state.consecutive_no_progress = 0;
    let max_rounds_outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: None,
        next_goal_hint: None,
        no_progress: false,
    };
    assert_eq!(
        soft_budget_checkpoint_resume_reason(&max_rounds_state, &policy, &max_rounds_outcome),
        Some("agent_loop_max_rounds")
    );

    let mut no_progress_state = LoopState::new(4);
    no_progress_state.round_no = 2;
    no_progress_state.max_rounds = 4;
    no_progress_state.consecutive_no_progress = policy.no_progress_limit + 1;
    let no_progress_outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: None,
        next_goal_hint: None,
        no_progress: true,
    };
    assert_eq!(
        soft_budget_checkpoint_resume_reason(&no_progress_state, &policy, &no_progress_outcome),
        Some("agent_loop_no_progress_limit")
    );

    let terminal_outcome = RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("structured_respond_clarify".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };
    assert_eq!(
        soft_budget_checkpoint_resume_reason(&no_progress_state, &policy, &terminal_outcome),
        None
    );

    let error_outcome = RoundOutcome {
        executed_actions: 1,
        had_error: true,
        stop_signal: None,
        next_goal_hint: None,
        no_progress: false,
    };
    assert_eq!(
        soft_budget_checkpoint_resume_reason(&no_progress_state, &policy, &error_outcome),
        None
    );

    let mut provider_blocker_state = LoopState::new(1);
    provider_blocker_state.round_no = 1;
    provider_blocker_state.max_rounds = 1;
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
    let provider_outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };
    assert_eq!(
        recoverable_provider_blocker_resume_reason(&provider_blocker_state),
        Some("provider_blocker_wait_background")
    );
    assert_eq!(
        soft_budget_checkpoint_resume_reason(&provider_blocker_state, &policy, &provider_outcome),
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
fn recoverable_checkpoint_state_requires_machine_progress() {
    let empty = LoopState::new(2);
    assert!(!loop_state_has_recoverable_checkpoint_state(&empty));

    let mut with_step = LoopState::new(2);
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
