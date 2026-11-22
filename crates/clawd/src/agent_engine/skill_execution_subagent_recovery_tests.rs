use super::*;
use crate::agent_engine::subagent_runtime::{
    SUBAGENT_STOP_SIGNAL_CAPABILITY_POLICY_REJECTED, SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED,
};

#[test]
fn resumed_graph_settles_the_original_invocation_without_double_charging_or_losing_provenance() {
    let task = super::super::tests::test_task();
    let mut state = LoopState::new();
    state.round_no = 1;
    state.total_steps_executed = 2;
    state.tool_calls_total = 1;
    let args = json!({"action":"bounded_parallel_readonly","children":[{"node_id":"child-one"}]});
    state.task_observations.push(json!({"schema_version":2,"owner_layer":"subagent_runtime",
        "action":"subagent_child_task_enqueue","status":"waiting","child_task_ids":["child-one"],"global_step":3,"step_in_round":1}));
    record_subagent_step_execution(
        &task,
        &mut state,
        3,
        1,
        &args,
        "call_capability",
        None,
        "original",
    )
    .unwrap();
    state.executed_step_results[0].started_at = 17;
    state.capability_results[0].provenance["registry_generation"] = json!("generation-pinned");
    assert_eq!(resumed_subagent_step_number(&state), 3);
    state.task_observations.push(json!({"schema_version":2,"owner_layer":"subagent_runtime",
        "action":"subagent_child_task_merge_reused","status":"ready","child_task_ids":["child-one"],"global_step":3,"step_in_round":1}));
    for _ in 0..2 {
        record_subagent_step_execution(
            &task,
            &mut state,
            3,
            1,
            &json!({"action":"bounded_parallel_readonly"}),
            "checkpoint_child_graph_refresh",
            None,
            "refresh",
        )
        .unwrap();
        assert_eq!(state.executed_step_results.len(), 1);
        assert_eq!(state.capability_results.len(), 1);
        assert_eq!(state.subtask_results.len(), 1);
        assert_eq!(state.tool_calls_total, 1);
        assert_eq!(state.executed_step_results[0].started_at, 17);
        assert_eq!(
            state.capability_results[0].provenance["registry_generation"],
            "generation-pinned"
        );
        assert_eq!(
            state.capability_results[0].provenance["continuation_args"],
            args
        );
        assert_eq!(
            state.capability_results[0].data["output"]["status"],
            "ready"
        );
    }
    state.executed_step_results.clear();
    assert_eq!(resumed_subagent_step_number(&state), 3);
}

#[test]
fn resumed_graph_fallback_does_not_reuse_an_unrelated_step_id() {
    let mut state = LoopState::new();
    state.total_steps_executed = 2;
    state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_8".to_string(),
            skill: "fixture_reader".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: None,
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    assert_eq!(resumed_subagent_step_number(&state), 9);
}

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
        "subagent_runtime_disabled",
        "unknown_rejection",
    ] {
        assert_eq!(
            normalize_subagent_stop_signal(Some(signal.into())),
            (Some(signal.into()), false)
        );
    }
    assert_eq!(normalize_subagent_stop_signal(None), (None, false));
}

#[test]
fn invalid_pre_dispatch_contract_and_context_arguments_replan_within_existing_recovery() {
    for signal in [
        crate::agent_engine::subagent_runtime::SUBAGENT_STOP_SIGNAL_INVALID_RESULT_CONTRACT,
        crate::agent_engine::subagent_runtime::SUBAGENT_STOP_SIGNAL_INVALID_ARGUMENT_CONSTRAINTS,
        crate::agent_engine::subagent_runtime::SUBAGENT_STOP_SIGNAL_UNSUPPORTED_CONTEXT_BUDGET,
    ] {
        assert_eq!(
            normalize_subagent_stop_signal(Some(signal.into())),
            (Some("recoverable_failure_continue_round".into()), true)
        );
        let task = super::super::tests::test_task();
        let mut state = LoopState::new();
        state
            .task_observations
            .push(json!({"schema_version":1,"owner_layer":"subagent_runtime",
            "status":"rejected","error_code":signal,"message_key":"subagent.admission_rejected",
            "source_skill":"subagent","retryable":true,"side_effect_applied":false,
            "field_issues":[{"path":"/result_contract/schema_version","expected":1}],
            "global_step":1,"step_in_round":1}));
        record_subagent_step_execution(
            &task,
            &mut state,
            1,
            1,
            &json!({"children":[]}),
            "call_capability",
            Some(signal),
            "invalid-schema",
        )
        .unwrap();
        assert!(state.has_recoverable_failure_context);
        assert_eq!(state.attempt_ledger_entries.len(), 1);
        assert!(state.successful_action_fingerprints.is_empty());
        let error: Value =
            serde_json::from_str(state.executed_step_results[0].error.as_deref().unwrap()).unwrap();
        assert_eq!(error["error_code"], signal);
        assert_eq!(error["status"], "error");
        assert_eq!(error["field_issues"][0]["expected"], 1);
        assert!(
            state.capability_results[0]
                .error
                .as_ref()
                .unwrap()
                .retryable
        );
    }
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
