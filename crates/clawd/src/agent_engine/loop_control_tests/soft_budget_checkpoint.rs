use super::super::soft_budget_checkpoint_resume_reason;
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
}
