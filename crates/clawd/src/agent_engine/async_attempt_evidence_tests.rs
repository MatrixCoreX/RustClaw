use super::*;
use claw_core::capability_result::{
    CapabilityResultEnvelope, CapabilityResultStatus, Continuation, ContinuationKind,
};

fn pending_evidence() -> (TaskCheckpoint, Value) {
    let mut checkpoint = poll_checkpoint();
    let mut loop_state = crate::agent_engine::LoopState::new();
    let prior = CapabilityResultEnvelope::ok("task.plan_set", None, json!({"revision": 1}));
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut loop_state,
        "task.plan_set",
        "{}",
        crate::executor::StepExecutionStatus::Ok,
        "previous_plan_result",
        None,
        "completed",
    );
    let previous_attempt =
        crate::agent_engine::attempt_ledger::build_attempt_ledger_snapshot(&loop_state).unwrap()[0]
            .clone();
    let mut pending = CapabilityResultEnvelope::ok(
        "dependency.install",
        None,
        json!({"job_id": "provider:video:1"}),
    );
    pending.status = CapabilityResultStatus::Waiting;
    pending.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("provider:video:1".to_string()),
        poll_after_ms: Some(2_000),
        state: json!({}),
    });
    pending.provenance = json!({"step_id": "step-1", "task_id": "task-1"});
    loop_state.capability_results = vec![prior, pending.clone()];
    let observation =
        crate::agent_engine::observed_output::structured_capability_observation(&pending).unwrap();
    assert!(observation.contains("dependency.install"));
    assert!(observation.contains("waiting"));
    assert!(!observation.contains("previous_plan_result"));
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut loop_state,
        "dependency.install",
        "{}",
        crate::executor::StepExecutionStatus::Ok,
        &observation,
        None,
        "async_job_pending",
    );
    let entry = loop_state.attempt_ledger_entries.last_mut().unwrap();
    entry.execution_step_id = Some("step-1".to_string());
    entry.async_job_id = Some("provider:video:1".to_string());
    entry.status = "waiting".to_string();
    checkpoint.attempt_ledger =
        crate::agent_engine::attempt_ledger::build_attempt_ledger_snapshot(&loop_state);
    checkpoint.capability_results = loop_state.capability_results;
    (checkpoint, previous_attempt)
}

#[test]
fn async_completion_replaces_only_matching_attempt_and_preserves_receipt() {
    let (checkpoint, previous_attempt) = pending_evidence();
    let completed = json!({"status": "ok", "text": "", "extra": {
        "schema_version": 1, "operation_receipt": {"verified": true, "installed_files": ["module.py"]}
    }});
    let result =
        completed_async_job_continuation_result("ask", "task-1", &checkpoint, &completed, 100)
            .unwrap();
    let successor = crate::task_lifecycle::task_checkpoint_from_result_json(&result).unwrap();
    assert_eq!(
        successor.attempt_ledger.as_ref().unwrap()[0],
        previous_attempt
    );
    let attempt = &successor.attempt_ledger.as_ref().unwrap()[1];
    assert_eq!(attempt["status"], "ok");
    assert!(attempt["observed_output"]
        .as_str()
        .unwrap()
        .contains("operation_receipt"));
    assert!(attempt["observed_output"]
        .as_str()
        .unwrap()
        .contains("module.py"));
    let mut resumed = crate::agent_engine::LoopState::new();
    crate::agent_engine::loop_state_seed::seed_loop_state_from_task_checkpoint(
        &mut resumed,
        &successor,
    );
    assert!(resumed.attempt_ledger_entries[1]
        .observed_output
        .contains("operation_receipt"));
    assert_eq!(resumed.tool_calls_total, 1);
    assert_eq!(
        successor.completed_side_effect_refs,
        checkpoint.completed_side_effect_refs
    );
    assert!(
        completed_async_job_continuation_result("ask", "task-1", &successor, &completed, 101)
            .is_none()
    );
}

#[test]
fn async_failure_exposes_structured_error_without_rewriting_prior_success() {
    let (checkpoint, previous_attempt) = pending_evidence();
    let failed = json!({"status": "error", "extra": {
        "error_code": "dependency_network_unavailable", "message_key": "skill.dependency.network_unavailable", "retryable": true
    }});
    let result =
        failed_async_job_continuation_result("ask", "task-1", &checkpoint, &failed, 100).unwrap();
    let successor = crate::task_lifecycle::task_checkpoint_from_result_json(&result).unwrap();
    let attempts = successor.attempt_ledger.unwrap();
    assert_eq!(attempts[0], previous_attempt);
    assert_eq!(attempts[1]["status"], "error");
    assert_eq!(attempts[1]["error_code"], "dependency_network_unavailable");
    assert_eq!(attempts[1]["retryable"], true);
    assert!(attempts[1]["observed_output"]
        .as_str()
        .unwrap()
        .contains("dependency_network_unavailable"));
}

#[test]
fn mismatched_async_identity_does_not_settle_other_attempts() {
    for field in ["async_job_id", "execution_step_id"] {
        let (mut checkpoint, _) = pending_evidence();
        checkpoint.attempt_ledger.as_mut().unwrap()[1][field] = json!("another-execution");
        let before = checkpoint.attempt_ledger.clone();
        let result = completed_async_job_continuation_result(
            "ask",
            "task-1",
            &checkpoint,
            &json!({"status": "ok"}),
            100,
        )
        .unwrap();
        let successor = crate::task_lifecycle::task_checkpoint_from_result_json(&result).unwrap();
        assert_eq!(successor.attempt_ledger, before);
    }
}

#[test]
fn completion_does_not_overwrite_a_later_step() {
    let (mut checkpoint, _) = pending_evidence();
    let unrelated = json!({"step_id": "step-2", "status": "ok", "output": "unrelated"});
    checkpoint.boundary_context["agent_loop_resume_state"]["executed_step_results"]
        .as_array_mut()
        .unwrap()
        .push(unrelated.clone());
    let result = completed_async_job_continuation_result(
        "ask",
        "task-1",
        &checkpoint,
        &json!({"status": "ok", "text": "actual_result"}),
        100,
    )
    .unwrap();
    let successor = crate::task_lifecycle::task_checkpoint_from_result_json(&result).unwrap();
    let steps = &successor.boundary_context["agent_loop_resume_state"]["executed_step_results"];
    assert_eq!(steps[1], unrelated);
    assert!(steps[0]["output"]
        .as_str()
        .unwrap()
        .contains("actual_result"));
}
