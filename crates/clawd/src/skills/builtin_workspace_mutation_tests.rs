use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    atomic_write_file, rewind_structured_mutation, run_authorized_mutation,
    run_checkpointed_workspace_mutation, structured_mutation_diff,
};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-workspace-mutation-{label}-{}",
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
fn atomic_write_replaces_content_without_leaving_temporary_files() {
    let workspace = TestWorkspace::new("atomic-write");
    let path = workspace.path().join("document.txt");
    fs::write(&path, "before").expect("seed file");

    atomic_write_file(&path, b"after").expect("atomic write");

    assert_eq!(fs::read_to_string(&path).expect("read file"), "after");
    let temporary_files = fs::read_dir(workspace.path())
        .expect("read workspace")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".agent-runtime-write-")
        })
        .count();
    assert_eq!(temporary_files, 0);
}

#[test]
fn atomic_write_failure_removes_temporary_file() {
    let workspace = TestWorkspace::new("atomic-write-cleanup");
    let target = workspace.path().join("existing-directory");
    fs::create_dir(&target).expect("create conflicting directory");

    atomic_write_file(&target, b"after").expect_err("rename over directory must fail");

    let temporary_files = fs::read_dir(workspace.path())
        .expect("read workspace")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".agent-runtime-write-")
        })
        .count();
    assert_eq!(temporary_files, 0);
    assert!(target.is_dir());
}

#[cfg(unix)]
#[test]
fn atomic_write_preserves_existing_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = TestWorkspace::new("atomic-write-mode");
    let path = workspace.path().join("script.sh");
    fs::write(&path, "#!/bin/sh\n").expect("seed script");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).expect("set mode");

    atomic_write_file(&path, b"#!/bin/sh\nexit 0\n").expect("atomic write");

    let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o750);
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

#[cfg(unix)]
#[test]
fn configured_workspace_root_alias_rebases_targets_to_the_canonical_root() {
    use std::os::unix::fs::symlink;

    let workspace = TestWorkspace::new("configured-root-alias-target");
    let alias_parent = TestWorkspace::new("configured-root-alias-parent");
    let alias_root = alias_parent.path().join("workspace");
    symlink(workspace.path(), &alias_root).expect("create configured workspace alias");
    let target = alias_root.join("notes.txt");
    fs::write(workspace.path().join("notes.txt"), "before").expect("seed target");

    let output = run_checkpointed_workspace_mutation(
        &alias_root,
        "task-configured-root-alias",
        "write_text",
        &target,
        || fs::write(&target, "after").map_err(|error| error.to_string()),
    )
    .expect("write through configured workspace alias");
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes.txt")).expect("changed target"),
        "after"
    );

    rewind_structured_mutation(&alias_root, &checkpoint_id(&output))
        .expect("rewind through configured workspace alias");
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes.txt")).expect("restored target"),
        "before"
    );
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

#[test]
fn authorized_host_scope_mutation_is_applied_without_workspace_checkpoint() {
    let workspace = TestWorkspace::new("host-scope-workspace");
    let outside = TestWorkspace::new("host-scope-outside");
    let path = outside.path().join("nested/result.txt");

    let output = run_authorized_mutation(
        workspace.path(),
        "task-host-scope",
        "write_text",
        &path,
        true,
        || {
            fs::create_dir_all(path.parent().expect("parent"))
                .map_err(|error| error.to_string())?;
            atomic_write_file(&path, b"host scope").map_err(|error| error.to_string())
        },
    )
    .expect("host-scope mutation");

    let value: Value = serde_json::from_str(&output).expect("output json");
    assert_eq!(
        value.get("source").and_then(Value::as_str),
        Some("host_scope_mutation")
    );
    assert_eq!(
        value.get("authority_scope").and_then(Value::as_str),
        Some("host_policy_grant")
    );
    assert_eq!(
        value.get("reversible").and_then(Value::as_bool),
        Some(false)
    );
    assert!(value.get("checkpoint_id").is_none());
    assert_eq!(fs::read_to_string(path).expect("host file"), "host scope");
}

#[test]
fn host_scope_mutation_requires_server_authorization() {
    let workspace = TestWorkspace::new("denied-host-scope-workspace");
    let outside = TestWorkspace::new("denied-host-scope-outside");
    let path = outside.path().join("denied.txt");

    let error = run_authorized_mutation(
        workspace.path(),
        "task-denied-host-scope",
        "write_text",
        &path,
        false,
        || fs::write(&path, "must not run").map_err(|error| error.to_string()),
    )
    .expect_err("host scope must require authorization");

    assert!(error.contains("invalid_target"));
    assert!(!path.exists());

    let parsed = crate::skills::parse_structured_skill_error(&error)
        .expect("structured pre-dispatch denial");
    let extra = parsed.extra.expect("canonical error extra");
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    assert!(crate::skills::structured_skill_error_requests_replan(
        &error
    ));
}

#[test]
fn runtime_state_target_is_rejected_as_retryable_before_operation() {
    let workspace = TestWorkspace::new("runtime-state-target");
    let target = workspace
        .path()
        .join(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
        .join("generated")
        .join("result.txt");
    let operation_ran = std::cell::Cell::new(false);

    let error = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-runtime-state-target",
        "write_text",
        &target,
        || {
            operation_ran.set(true);
            fs::write(&target, "must not run").map_err(|error| error.to_string())
        },
    )
    .expect_err("runtime state target must be rejected");

    assert!(!operation_ran.get());
    assert!(!target.exists());
    let parsed =
        crate::skills::parse_structured_skill_error(&error).expect("structured target rejection");
    assert_eq!(parsed.error_code, "invalid_target_path");
    let extra = parsed.extra.expect("canonical error extra");
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    assert!(crate::skills::structured_skill_error_proves_not_applied(
        &error
    ));
}

#[test]
fn verified_unchanged_operation_error_is_marked_no_effect() {
    let workspace = TestWorkspace::new("verified-operation-no-effect");
    let target = workspace.path().join("result.txt");
    let operation_error = crate::skills::structured_skill_error_from_parts(
        "write_file",
        "permission_denied",
        "write denied",
        None,
        None,
    );

    let error = run_checkpointed_workspace_mutation(
        workspace.path(),
        "task-operation-no-effect",
        "write_text",
        &target,
        || Err(operation_error),
    )
    .expect_err("operation must fail");

    let parsed =
        crate::skills::parse_structured_skill_error(&error).expect("structured operation error");
    let extra = parsed.extra.expect("canonical error extra");
    assert_eq!(extra["failure_phase"], "execution_no_effect");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["retryable"], false);
    assert!(crate::skills::structured_skill_error_proves_not_applied(
        &error
    ));
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
