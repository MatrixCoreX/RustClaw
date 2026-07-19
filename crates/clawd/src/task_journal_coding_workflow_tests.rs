use serde_json::{json, Value};

use super::TaskJournal;

fn step_result(
    step_id: &str,
    skill: &str,
    status: crate::executor::StepExecutionStatus,
    output: Option<String>,
    error: Option<String>,
) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status,
        output,
        error,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn summary_json_includes_coding_workflow_verified_contract() {
    let mut journal = TaskJournal::for_task("task-coding-workflow", "ask", "change and verify");
    journal.push_step_result(&step_result(
        "step_1",
        "fs_basic",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "action": "write_text",
                    "path": "src/lib.rs",
                    "resolved_path": "/workspace/src/lib.rs",
                    "planned_change": "add library entrypoint",
                    "diff_ref": "diff:src/lib.rs:step_1"
                }
            })
            .to_string(),
        ),
        None,
    ));
    journal.push_step_result(&step_result(
        "step_2",
        "run_cmd",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "command": "cargo test -p clawd"
                }
            })
            .to_string(),
        ),
        None,
    ));

    let summary = journal.to_summary_json();
    let workflow = summary.get("coding_workflow").expect("coding workflow");

    assert_eq!(workflow["planned_change_count"], 1);
    assert_eq!(workflow["planned_changes"][0], "add library entrypoint");
    assert_eq!(workflow["diff_ref_count"], 1);
    assert_eq!(workflow["diff_refs"][0], "diff:src/lib.rs:step_1");
    assert_eq!(workflow["changed_file_count"], 1);
    assert_eq!(workflow["changed_files"][0], "src/lib.rs");
    assert_eq!(workflow["verification_command_count"], 1);
    assert_eq!(workflow["verification_commands"][0], "cargo test -p clawd");
    assert_eq!(workflow["verification_status"], "verified");
    assert_eq!(workflow["current_phase_hint"], "summarize");
    assert_eq!(workflow["next_step"], "summarize");
    assert!(workflow
        .get("checkpoint_refs")
        .and_then(Value::as_array)
        .is_some_and(|refs| refs.iter().any(|value| value
            .as_str()
            .is_some_and(|item| item == "coding_checkpoint:verification_command:step_2"))));
    assert_eq!(
        workflow
            .pointer("/done_condition_coverage/1/status")
            .and_then(Value::as_str),
        Some("verified")
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/gate_status")
            .and_then(Value::as_str),
        Some("satisfied")
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/can_report_fully_verified")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn summary_json_includes_coding_workflow_repair_contract() {
    let mut journal = TaskJournal::for_task("task-coding-workflow-repair", "ask", "fix tests");
    journal.push_step_result(&step_result(
        "step_1",
        "run_cmd",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some("exit=101 command=cargo test -p clawd\nstderr_ref=artifact:stderr:step_1".to_string()),
    ));

    let summary = journal.to_summary_json();
    let workflow = summary.get("coding_workflow").expect("coding workflow");

    assert_eq!(workflow["verification_status"], "failed");
    assert_eq!(workflow["next_step"], "repair_failed_verification");
    assert_eq!(workflow["failure_kinds"][0], "test");
    assert_eq!(workflow["failed_command_count"], 1);
    assert_eq!(workflow["failed_commands"][0], "cargo test -p clawd");
    assert_eq!(workflow["failed_command_ref_count"], 1);
    assert_eq!(workflow["failed_command_refs"][0], "step:step_1");
    assert_eq!(workflow["failed_command_stderr_ref_count"], 1);
    assert_eq!(
        workflow["failed_command_stderr_refs"][0],
        "artifact:stderr:step_1"
    );
    assert_eq!(workflow["repair_attempt_count"], 1);
    assert_eq!(workflow["repair_attempt_refs"][0], "step:step_1");
    assert!(workflow
        .get("remaining_risks")
        .and_then(Value::as_array)
        .is_some_and(|risks| risks
            .iter()
            .any(|value| value.as_str() == Some("failed_verification"))));
    assert_eq!(
        workflow
            .pointer("/validation_gate/gate_status")
            .and_then(Value::as_str),
        Some("repair_required")
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/can_report_fully_verified")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/repair_signal/signal_kind")
            .and_then(Value::as_str),
        Some("verification_failed")
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/repair_signal/failed_command_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/repair_signal/failed_command_stderr_ref_count")
            .and_then(Value::as_u64),
        Some(1)
    );
}

#[test]
fn latest_green_verification_supersedes_historical_red_result() {
    let mut journal = TaskJournal::for_task(
        "task-coding-workflow-red-green",
        "ask",
        "run red test, fix, and verify",
    );
    journal.push_step_result(&step_result(
        "step_red",
        "run_cmd",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some(
            "exit=1 command=python3 -m unittest test_calc_core.py -v\n\
             stderr_ref=artifact:stderr:step_red"
                .to_string(),
        ),
    ));
    journal.push_step_result(&step_result(
        "step_fix",
        "fs_basic",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "action": "write_text",
                    "path": "calc_core.py"
                }
            })
            .to_string(),
        ),
        None,
    ));
    journal.push_step_result(&step_result(
        "step_green",
        "run_cmd",
        crate::executor::StepExecutionStatus::Ok,
        Some("exit=0 command=python3 -m unittest test_calc_core.py -v 2>&1 | tail -50".to_string()),
        None,
    ));

    let workflow = journal.to_summary_json()["coding_workflow"].clone();
    assert_eq!(workflow["schema_version"], 2);
    assert_eq!(workflow["projection_revision"], 9);
    assert_eq!(workflow["latest_verification_step_ref"], "step_green");
    assert_eq!(workflow["verification_status"], "verified");
    assert_eq!(workflow["current_phase_hint"], "summarize");
    assert_eq!(workflow["failure_kind_count"], 0);
    assert_eq!(workflow["historical_failure_kind_count"], 1);
    assert_eq!(workflow["historical_failure_kinds"][0], "test");
    assert_eq!(workflow["validation_gate"]["gate_status"], "satisfied");

    let evidence = journal
        .event_stream_snapshot()
        .into_iter()
        .find(|event| event["event_type"] == "coding_evidence")
        .expect("coding evidence");
    assert_eq!(evidence["payload"]["schema_version"], 2);
    assert_eq!(evidence["payload"]["projection_step_count"], 3);
    assert_eq!(
        evidence["payload"]["latest_verification_step_ref"],
        "step_green"
    );
    assert_eq!(evidence["payload"]["changed_files"][0], "calc_core.py");
    assert_eq!(evidence["payload"]["verification_status"], "verified");
    assert_eq!(evidence["payload"]["failure_count"], 0);
    assert!(evidence["payload"]["failures"]
        .as_array()
        .is_some_and(Vec::is_empty));
    assert!(evidence["payload"]["historical_failure_count"]
        .as_u64()
        .is_some_and(|count| count > 0));
    assert_eq!(
        evidence["payload"]["historical_verification_failure_kinds"][0],
        "test"
    );
}

#[test]
fn summary_json_marks_changed_files_without_verification_as_gate_required() {
    let mut journal = TaskJournal::for_task("task-coding-workflow-unverified", "ask", "edit only");
    journal.push_step_result(&step_result(
        "step_1",
        "fs_basic",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "action": "write_text",
                    "path": "src/lib.rs",
                    "resolved_path": "/workspace/src/lib.rs"
                }
            })
            .to_string(),
        ),
        None,
    ));

    let summary = journal.to_summary_json();
    let workflow = summary.get("coding_workflow").expect("coding workflow");

    assert_eq!(workflow["verification_status"], "unverified");
    assert_eq!(
        workflow
            .pointer("/validation_gate/gate_status")
            .and_then(Value::as_str),
        Some("verification_required")
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/can_report_fully_verified")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        workflow
            .pointer("/validation_gate/requires_verification")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn summary_json_replays_persisted_apply_patch_step_without_observations() {
    let mut journal = TaskJournal::for_task("task-persisted-patch", "ask", "apply patch");
    journal.step_results.push(super::TaskJournalStepTrace::ok(
        "step_patch",
        "workspace_patch",
        json!({
            "extra": {
                "schema_version": 1,
                "source": "workspace_patch",
                "status": "ok",
                "action": "apply_patch",
                "patch_id": "sha256:patch-1",
                "checkpoint_id": "patch_checkpoint_1",
                "changed_files": ["src/lib.rs"]
            }
        })
        .to_string(),
    ));

    let summary = journal.to_summary_json();
    let workflow = summary.get("coding_workflow").expect("coding workflow");

    assert_eq!(journal.task_observations.len(), 0);
    assert_eq!(workflow["changed_file_count"], 1);
    assert_eq!(workflow["changed_files"][0], "src/lib.rs");
    assert_eq!(workflow["verification_status"], "unverified");
    assert_eq!(workflow["current_phase_hint"], "verify");
}

#[test]
fn summary_json_projects_read_only_workspace_diff_as_review() {
    let mut journal = TaskJournal::for_task("task-persisted-diff", "ask", "inspect diff");
    journal.step_results.push(super::TaskJournalStepTrace::ok(
        "step_diff",
        "workspace_patch",
        json!({
            "extra": {
                "schema_version": 1,
                "source": "workspace_patch",
                "status": "ok",
                "action": "diff",
                "patch_id": "sha256:patch-1",
                "checkpoint_id": "patch_checkpoint_1",
                "changed_files": ["src/lib.rs"],
                "patch": "diff --git a/src/lib.rs b/src/lib.rs\n"
            }
        })
        .to_string(),
    ));

    let workflow = journal.to_summary_json()["coding_workflow"].clone();
    assert_eq!(workflow["changed_file_count"], 0);
    assert_eq!(workflow["diff_ref_count"], 1);
    assert_eq!(workflow["diff_refs"][0], "sha256:patch-1");
    assert_eq!(workflow["verification_status"], "not_applicable");
    assert_eq!(workflow["current_phase_hint"], "review");
    assert_eq!(workflow["next_step"], "summarize");
}
