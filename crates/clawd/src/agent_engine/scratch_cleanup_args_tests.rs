use super::*;

fn ok_step(step_id: &str, skill: &str, output: &str) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn scratch_cleanup_requires_observed_write_in_same_root() {
    let workspace_root = Path::new("/workspace");
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"tmp/job-a/note.txt"}}"#,
    ));

    assert!(scratch_lifecycle_progress_has_write_in_root(
        workspace_root,
        &loop_state,
        "tmp/job-a"
    ));
    assert!(!scratch_lifecycle_progress_has_write_in_root(
        workspace_root,
        &loop_state,
        "tmp/job-b"
    ));
}

#[test]
fn scratch_cleanup_accepts_observed_archive_in_same_root() {
    let workspace_root = Path::new("/workspace");
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "archive_basic",
        r#"{"extra":{"action":"pack","archive":"tmp/job-a/result.zip"}}"#,
    ));

    assert!(scratch_lifecycle_progress_has_archive_pack_in_root(
        workspace_root,
        &loop_state,
        "tmp/job-a"
    ));
    assert!(!scratch_lifecycle_progress_has_archive_pack_in_root(
        workspace_root,
        &loop_state,
        "tmp/job-b"
    ));
}
