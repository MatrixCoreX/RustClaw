use super::*;
use crate::agent_engine::LoopState;
use crate::executor::{StepExecutionResult, StepExecutionStatus};

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
fn reusable_terminal_json_after_later_observation_preserves_prior_success_answer() {
    let terminal_answer = r#"{"created_files":["calc_core.py"],"test_status":"passed"}"#;
    let mut loop_state = LoopState::new();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "EXIT_CODE=0"));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "synthesize_answer", terminal_answer));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "respond", terminal_answer));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"calc_core.py","excerpt":"1|content"}}"#,
    ));

    assert_eq!(
        reusable_terminal_json_after_later_observation(&loop_state).as_deref(),
        Some(terminal_answer)
    );
}

#[test]
fn reusable_terminal_json_requires_later_nonterminal_observation() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "synthesize_answer",
        r#"{"status":"ok"}"#,
    ));

    assert!(reusable_terminal_json_after_later_observation(&loop_state).is_none());
}

#[test]
fn reusable_terminal_json_rejects_unresolved_machine_values() {
    for answer in [
        r#"{"test_status":"not_observed"}"#,
        r#"{"created_files":["<missing>"]}"#,
        r#"{"created_files":null}"#,
        r#"{"answer":"{{last_output}}"}"#,
        r#"{"steps":[]}"#,
    ] {
        let mut loop_state = LoopState::new();
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "synthesize_answer", answer));
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "fs_basic", r#"{"status":"ok"}"#));
        assert!(
            reusable_terminal_json_after_later_observation(&loop_state).is_none(),
            "unexpected reusable terminal JSON: {answer}"
        );
    }
}
