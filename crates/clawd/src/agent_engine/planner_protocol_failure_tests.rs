use super::*;
use crate::executor::{StepExecutionResult, StepExecutionStatus};

#[test]
fn exhausted_native_protocol_retains_successful_execution_and_its_owner() {
    for code in [
        "native_plan_contract_repair_stagnated",
        "native_plan_contract_repair_budget_exhausted",
        "native_plan_contract_repair_unavailable",
    ] {
        let mut state = LoopState::default();
        state.executed_step_results.push(StepExecutionResult {
            step_id: "write_1".into(),
            skill: "fixture_writer".into(),
            status: StepExecutionStatus::Ok,
            output: Some("completed fixture output".into()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });
        state
            .task_observations
            .push(json!({"owner_layer":"planner","state":"terminal",
            "reason_code":code,"source_error_code":"native_respond_items_not_array"}));
        assert!(retain_terminal_failure(&mut state, code));
        let payload = terminal_failure_payload(&state).unwrap();
        assert_eq!(payload["failure_owner"], "planner_protocol");
        assert_eq!(payload["execution_failure"], false);
        assert_eq!(payload["completed_step_ids"], json!(["write_1"]));
        assert!(state.executed_step_results[0].is_ok());
        assert_eq!(
            state.executed_step_results[0].output.as_deref(),
            Some("completed fixture output")
        );
    }
}

#[test]
fn unowned_or_nonterminal_errors_cannot_claim_native_protocol_failure() {
    let code = "native_plan_contract_repair_stagnated";
    let mut state = LoopState::default();
    assert!(!retain_terminal_failure(&mut state, code));
    for (owner, phase) in [("tool", "terminal"), ("planner", "progress")] {
        state.task_observations =
            vec![json!({"owner_layer":owner,"state":phase,"reason_code":code})];
        assert!(!retain_terminal_failure(&mut state, code));
    }
    state.task_observations =
        vec![json!({"owner_layer":"planner","state":"terminal","reason_code":code})];
    assert!(!retain_terminal_failure(
        &mut state,
        "unrelated_provider_failure"
    ));
    assert!(state.last_stop_signal.is_none());
    state.last_stop_signal = Some("respond_terminal".into());
    assert!(terminal_failure_payload(&state).is_none());
}
