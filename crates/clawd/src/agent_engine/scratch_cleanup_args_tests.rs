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
    let mut loop_state = LoopState::new();
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
    let mut loop_state = LoopState::new();
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

#[test]
fn scratch_cleanup_does_not_treat_a_top_level_file_as_its_own_directory() {
    for (skill, output) in [
        (
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"tmp/job-a"}}"#,
        ),
        (
            "archive_basic",
            r#"{"extra":{"action":"pack","archive":"tmp/job-a"}}"#,
        ),
    ] {
        let mut loop_state = LoopState::new();
        loop_state
            .executed_step_results
            .push(ok_step("step_1", skill, output));
        let mut args = serde_json::json!({"path":"tmp/job-a"});
        let original = args.clone();
        assert!(!enrich_scratch_filesystem_cleanup_runtime_args(
            Path::new("/workspace"),
            &loop_state,
            "fs_basic",
            &serde_json::json!({"action":"remove_path","path":"tmp/job-a"}),
            "remove_file",
            &mut args,
        ));
        assert_eq!(args, original);
    }
}

#[test]
fn scratch_cleanup_only_enriches_an_observed_parent_without_expanding_explicit_limits() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"tmp/job-a/note.txt"}}"#,
    ));
    for (supplied, enriched) in [
        (serde_json::json!({"path":"tmp/job-a"}), true),
        (
            serde_json::json!({"path":"tmp/job-a","recursive":false}),
            false,
        ),
        (
            serde_json::json!({"path":"tmp/job-a","target_kind":"file"}),
            false,
        ),
        (serde_json::json!({"path":"tmp/job-b"}), false),
    ] {
        let mut args = supplied.clone();
        assert_eq!(
            enrich_scratch_filesystem_cleanup_runtime_args(
                Path::new("/workspace"),
                &loop_state,
                "fs_basic",
                &serde_json::json!({"action":"remove_path","path": supplied["path"]}),
                "remove_file",
                &mut args,
            ),
            enriched
        );
        if enriched {
            assert_eq!(args["target_kind"], "directory");
            assert_eq!(args["recursive"], true);
        } else {
            assert_eq!(args, supplied);
        }
    }
}

#[test]
fn scratch_descendant_checks_use_components_not_path_prefixes() {
    let root = Path::new("/workspace");
    assert!(is_descendant_of_scratch_root(
        root,
        "/workspace/tmp/job-a/note.txt",
        "tmp/job-a"
    ));
    for path in [
        "tmp/job-a",
        "tmp/job-ab/note.txt",
        "tmp/job-a/../outside",
        "/other/tmp/job-a/file",
        r"tmp\job-a\..\outside",
        r"tmp\job-a\note.txt",
        r"tmp/job-a\child.txt",
    ] {
        assert!(
            !is_descendant_of_scratch_root(root, path, "tmp/job-a"),
            "{path}"
        );
    }
}
