use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    rewind_structured_mutation, run_checkpointed_workspace_mutation, structured_mutation_diff,
};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-workspace-mutation-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("create workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn checkpoint_id(output: &str) -> String {
    serde_json::from_str::<Value>(output)
        .expect("structured mutation output")
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .expect("checkpoint id")
        .to_string()
}

#[test]
fn existing_file_write_can_be_rewound_with_compensation_evidence() {
    let workspace = TestWorkspace::new("existing-file");
    let path = workspace.path().join("src/lib.rs");
    fs::create_dir_all(path.parent().expect("parent")).expect("create src");
    fs::write(&path, "before\n").expect("seed file");

    let output = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-1",
        "write_text",
        &path,
        || fs::write(&path, "after\n").map_err(|error| error.to_string()),
    )
    .expect("write with checkpoint");
    let value: Value = serde_json::from_str(&output).expect("output json");
    assert_eq!(
        value.get("source").and_then(Value::as_str),
        Some("workspace_mutation")
    );
    assert_eq!(value.get("state").and_then(Value::as_str), Some("applied"));
    assert_eq!(value.get("reversible").and_then(Value::as_bool), Some(true));
    let checkpoint_id = checkpoint_id(&output);

    let rewind = rewind_structured_mutation(workspace.path(), &checkpoint_id)
        .expect("rewind structured mutation");
    let rewind: Value = serde_json::from_str(&rewind).expect("rewind json");
    assert_eq!(
        rewind
            .get("compensates_checkpoint_id")
            .and_then(Value::as_str),
        Some(checkpoint_id.as_str())
    );
    assert_eq!(fs::read_to_string(path).expect("restored file"), "before\n");
}

#[test]
fn created_file_rewind_removes_empty_created_parents() {
    let workspace = TestWorkspace::new("created-file");
    let path = workspace.path().join("generated/nested/result.txt");
    let output = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-2",
        "write_text",
        &path,
        || {
            fs::create_dir_all(path.parent().expect("parent"))
                .map_err(|error| error.to_string())?;
            fs::write(&path, "created").map_err(|error| error.to_string())
        },
    )
    .expect("create with checkpoint");

    rewind_structured_mutation(workspace.path(), &checkpoint_id(&output)).expect("rewind creation");
    assert!(!path.exists());
    assert!(!workspace.path().join("generated").exists());
}

#[test]
fn recursive_directory_removal_can_be_restored() {
    let workspace = TestWorkspace::new("removed-directory");
    let target = workspace.path().join("tree");
    fs::create_dir_all(target.join("nested")).expect("create tree");
    fs::write(target.join("a.txt"), "alpha").expect("seed alpha");
    fs::write(target.join("nested/b.txt"), "beta").expect("seed beta");
    let output = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-3",
        "remove_path",
        &target,
        || fs::remove_dir_all(&target).map_err(|error| error.to_string()),
    )
    .expect("remove with checkpoint");
    assert!(!target.exists());

    let diff = structured_mutation_diff(workspace.path(), &checkpoint_id(&output))
        .expect("structured checkpoint diff");
    let diff: Value = serde_json::from_str(&diff).expect("diff json");
    assert_eq!(
        diff.get("diff_available").and_then(Value::as_bool),
        Some(false)
    );
    rewind_structured_mutation(workspace.path(), &checkpoint_id(&output)).expect("rewind removal");
    assert_eq!(
        fs::read_to_string(target.join("nested/b.txt")).expect("restored beta"),
        "beta"
    );
}

#[test]
fn later_user_edit_blocks_rewind() {
    let workspace = TestWorkspace::new("later-user-edit");
    let path = workspace.path().join("notes.txt");
    fs::write(&path, "before").expect("seed file");
    let output = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-4",
        "write_text",
        &path,
        || fs::write(&path, "agent edit").map_err(|error| error.to_string()),
    )
    .expect("write with checkpoint");
    fs::write(&path, "user edit").expect("later user edit");

    let error = rewind_structured_mutation(workspace.path(), &checkpoint_id(&output))
        .expect_err("rewind must reject changed target");
    assert!(error.contains("rewind_precondition_failed"));
    assert_eq!(
        fs::read_to_string(path).expect("user edit retained"),
        "user edit"
    );
}

#[test]
fn failed_operation_restores_partial_mutation() {
    let workspace = TestWorkspace::new("failed-operation");
    let path = workspace.path().join("partial.txt");
    fs::write(&path, "before").expect("seed file");

    let error = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-5",
        "write_text",
        &path,
        || {
            fs::write(&path, "partial").map_err(|error| error.to_string())?;
            Err("operation_failed".to_string())
        },
    )
    .expect_err("operation must fail");
    assert_eq!(error, "operation_failed");
    assert_eq!(fs::read_to_string(path).expect("restored file"), "before");
}

#[test]
fn identical_file_write_is_recorded_as_no_op() {
    let workspace = TestWorkspace::new("no-op");
    let path = workspace.path().join("same.txt");
    fs::write(&path, "same").expect("seed file");
    let output = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-no-op",
        "write_text",
        &path,
        || fs::write(&path, "same").map_err(|error| error.to_string()),
    )
    .expect("no-op write");
    let value: Value = serde_json::from_str(&output).expect("output json");
    assert_eq!(value.get("state").and_then(Value::as_str), Some("no_op"));
    assert_eq!(
        value.get("reversible").and_then(Value::as_bool),
        Some(false)
    );
}

#[cfg(unix)]
#[test]
fn symlink_target_is_denied_before_mutation() {
    use std::os::unix::fs::symlink;

    let workspace = TestWorkspace::new("symlink-workspace");
    let outside = TestWorkspace::new("symlink-outside");
    let link = workspace.path().join("linked");
    symlink(outside.path(), &link).expect("create symlink");
    let target = link.join("value.txt");
    let error = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-6",
        "write_text",
        &target,
        || fs::write(&target, "denied").map_err(|error| error.to_string()),
    )
    .expect_err("symlink must be denied");
    assert!(error.contains("symlink_denied"));
    assert!(!outside.path().join("value.txt").exists());
}
