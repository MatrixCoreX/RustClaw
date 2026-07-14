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
                    "resolved_path": "/workspace/src/lib.rs"
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
}

#[test]
fn summary_json_includes_coding_workflow_repair_contract() {
    let mut journal = TaskJournal::for_task("task-coding-workflow-repair", "ask", "fix tests");
    journal.push_step_result(&step_result(
        "step_1",
        "run_cmd",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some("exit=101 command=cargo test -p clawd".to_string()),
    ));

    let summary = journal.to_summary_json();
    let workflow = summary.get("coding_workflow").expect("coding workflow");

    assert_eq!(workflow["verification_status"], "failed");
    assert_eq!(workflow["next_step"], "repair_failed_verification");
    assert_eq!(workflow["failure_kinds"][0], "test");
    assert_eq!(workflow["repair_attempt_count"], 1);
    assert_eq!(workflow["repair_attempt_refs"][0], "step:step_1");
    assert!(workflow
        .get("remaining_risks")
        .and_then(Value::as_array)
        .is_some_and(|risks| risks
            .iter()
            .any(|value| value.as_str() == Some("failed_verification"))));
}
