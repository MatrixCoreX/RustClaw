use super::*;

#[test]
fn publishable_synthesis_marks_terminal_answer_ready() {
    let mut loop_state = LoopState::new();
    loop_state.last_publishable_synthesis_output = Some("observed summary".to_string());

    assert_eq!(
        terminal_user_answer_stop_signal(&loop_state),
        Some("terminal_user_answer_ready")
    );
}

#[test]
fn empty_terminal_answer_does_not_stop() {
    let mut loop_state = LoopState::new();
    loop_state.last_publishable_synthesis_output = Some("   ".to_string());

    assert_eq!(terminal_user_answer_stop_signal(&loop_state), None);
}
