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
                    "resolved_path": "/workspace/src/lib.rs"
                }
            })
            .to_string(),
        ),
        None,
    ));

    let observation = journal
        .task_observations
        .iter()
        .find(|value| {
            value.get("kind").and_then(serde_json::Value::as_str) == Some("coding_state_transition")
        })
        .expect("coding transition observation");
    assert_eq!(observation["phase"], "edit");
    assert_eq!(observation["next_phase_hint"], "verify");
    assert_eq!(observation["status"], "ok");
    assert_eq!(observation["action"], "write_text");
    assert_eq!(observation["changed_files"][0], "src/lib.rs");
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
        Some("exit=101 command=cargo test -p clawd".to_string()),
    ));

    let observation = journal
        .task_observations
        .iter()
        .find(|value| {
            value.get("kind").and_then(serde_json::Value::as_str) == Some("coding_state_transition")
        })
        .expect("coding transition observation");
    assert_eq!(observation["phase"], "repair");
    assert_eq!(observation["next_phase_hint"], "repair");
    assert_eq!(observation["status"], "error");
    assert_eq!(observation["command"], "cargo test -p clawd");
    assert_eq!(observation["verification_command"], "cargo test -p clawd");
    assert_eq!(observation["failure_kind"], "test");
}
