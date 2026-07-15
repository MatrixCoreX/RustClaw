use serde_json::json;

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
        started_at: 10,
        finished_at: 12,
    }
}

fn observation<'a>(
    journal: &'a crate::task_journal::TaskJournal,
    kind: &str,
) -> &'a serde_json::Value {
    journal
        .task_observations
        .iter()
        .find(|value| value.get("kind").and_then(serde_json::Value::as_str) == Some(kind))
        .expect("expected observation")
}

#[test]
fn step_result_records_coding_edit_transition_observation() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-coding-state-edit",
        "ask",
        "change a file and verify",
    );
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

    let transition = observation(&journal, "coding_state_transition");
    assert_eq!(transition["phase"], "edit");
    assert_eq!(transition["next_phase_hint"], "verify");
    assert_eq!(transition["status"], "ok");
    assert_eq!(transition["action"], "write_text");
    assert_eq!(transition["planned_changes"][0], "add library entrypoint");
    assert_eq!(transition["diff_refs"][0], "diff:src/lib.rs:step_1");
    assert_eq!(transition["changed_files"][0], "src/lib.rs");

    let checkpoint = observation(&journal, "coding_checkpoint");
    assert_eq!(checkpoint["checkpoint_kind"], "file_edit_group");
    assert_eq!(checkpoint["verification_status"], "unverified");
    assert_eq!(checkpoint["planned_changes"][0], "add library entrypoint");
    assert_eq!(checkpoint["diff_refs"][0], "diff:src/lib.rs:step_1");
    assert_eq!(checkpoint["changed_files"][0], "src/lib.rs");
}

#[test]
fn step_result_records_failed_verification_transition_observation() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-coding-state-verify-fail",
        "ask",
        "run tests",
    );
    journal.push_step_result(&step_result(
        "step_2",
        "run_cmd",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some("exit=101 command=cargo test -p clawd\nstderr_ref=artifact:stderr:step_2".to_string()),
    ));

    let transition = observation(&journal, "coding_state_transition");
    assert_eq!(transition["phase"], "repair");
    assert_eq!(transition["next_phase_hint"], "repair");
    assert_eq!(transition["status"], "error");
    assert_eq!(transition["command"], "cargo test -p clawd");
    assert_eq!(transition["verification_command"], "cargo test -p clawd");
    assert_eq!(transition["failed_commands"][0], "cargo test -p clawd");
    assert_eq!(transition["failed_command_refs"][0], "step:step_2");
    assert_eq!(
        transition["failed_command_stderr_refs"][0],
        "artifact:stderr:step_2"
    );
    assert_eq!(transition["failure_kind"], "test");

    let checkpoint = observation(&journal, "coding_checkpoint");
    assert_eq!(checkpoint["checkpoint_kind"], "failed_step");
    assert_eq!(checkpoint["verification_status"], "failed");
    assert_eq!(checkpoint["failed_commands"][0], "cargo test -p clawd");
    assert_eq!(checkpoint["failed_command_refs"][0], "step:step_2");
    assert_eq!(
        checkpoint["failed_command_stderr_refs"][0],
        "artifact:stderr:step_2"
    );
    assert_eq!(checkpoint["failure_kind"], "test");
}

#[test]
fn edit_after_repair_records_fix_applied_checkpoint() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-coding-state-fix",
        "ask",
        "fix failing test",
    );
    journal.push_step_result(&step_result(
        "step_1",
        "run_cmd",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some("exit=101 command=cargo test -p clawd".to_string()),
    ));
    journal.push_step_result(&step_result(
        "step_2",
        "fs_basic",
        crate::executor::StepExecutionStatus::Ok,
        Some(
            json!({
                "extra": {
                    "action": "write_text",
                    "path": "src/lib.rs"
                }
            })
            .to_string(),
        ),
        None,
    ));

    let checkpoints = journal
        .task_observations
        .iter()
        .filter(|value| {
            value.get("kind").and_then(serde_json::Value::as_str) == Some("coding_checkpoint")
        })
        .collect::<Vec<_>>();
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[1]["checkpoint_kind"], "fix_applied");
    assert_eq!(checkpoints[1]["changed_files"][0], "src/lib.rs");
}
