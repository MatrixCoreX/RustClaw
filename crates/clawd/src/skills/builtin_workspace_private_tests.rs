use super::*;
use crate::runtime::host_filesystem::HostFilesystemPolicy;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

fn private_state(root: &Path, private: &Path) -> AppState {
    let mut state = test_state(root.to_path_buf());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private.to_path_buf()], root).unwrap());
    state
}

fn private_directory(root: &Path) -> PathBuf {
    let path = root.join("private");
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    path
}

fn patch(path: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-before\n+after\n")
}

#[tokio::test]
async fn host_private_mutation_preview_refuses_private_preimages() {
    let root = TempDirGuard::new("private_mutation_preview");
    let private = private_directory(&root.path);
    fs::write(private.join("marker"), b"before\n").unwrap();
    fs::write(root.path.join("public"), b"before\n").unwrap();
    let state = private_state(&root.path, &private);
    let public = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"preview_replace_text", "path":"public",
            "old_text":"before", "new_text":"after"
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&public).unwrap()["would_change"],
        true
    );
    assert_eq!(fs::read(root.path.join("public")).unwrap(), b"before\n");
    let result = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"preview_replace_text", "path":"private/marker",
            "old_text":"before", "new_text":"after"
        }),
    )
    .await;
    eprintln!(
        "HOST_PRIVATE_MUTATION case=preview actual_builtin=true refused={} llm_calls=0",
        result.is_err()
    );
    assert!(
        result.is_err(),
        "private preview must not expose preimage evidence"
    );
    let parsed = crate::skills::parse_structured_skill_error(&result.unwrap_err()).unwrap();
    assert_eq!(parsed.error_code, "host_private_directory_access_denied");
    assert_eq!(parsed.extra.unwrap()["retryable"], false);
}

#[path = "builtin_workspace_private_controls_tests.rs"]
mod controls;

#[tokio::test]
async fn host_private_mutation_write_and_append_leave_private_targets_unchanged() {
    let root = TempDirGuard::new("private_mutation_write");
    let private = private_directory(&root.path);
    fs::write(private.join("marker"), b"before\n").unwrap();
    let state = private_state(&root.path, &private);
    let mut outcomes = Vec::new();
    for append in [false, true] {
        let result = execute_builtin_skill(
            &state,
            "write_file",
            &json!({
                "path":"private/marker", "content":"after", "append":append
            }),
        )
        .await;
        let unchanged = fs::read(private.join("marker")).unwrap() == b"before\n";
        outcomes.push((append, result.is_err(), unchanged));
        fs::write(private.join("marker"), b"before\n").unwrap();
    }
    let created = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path":"private/new/marker", "content":"after", "create_parents":true
        }),
    )
    .await;
    let no_parent = !private.join("new").exists();
    eprintln!("HOST_PRIVATE_MUTATION case=write actual_builtin=true outcomes={outcomes:?} create_refused={} parent_absent={no_parent} llm_calls=0", created.is_err());
    assert!(outcomes
        .iter()
        .all(|(_, refused, unchanged)| *refused && *unchanged));
    assert!(created.is_err() && no_parent);
}

#[tokio::test]
async fn host_private_mutation_patch_refuses_private_writes() {
    let root = TempDirGuard::new("private_mutation_patch");
    let private = private_directory(&root.path);
    fs::write(private.join("marker"), b"before\n").unwrap();
    let state = private_state(&root.path, &private);
    let result = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"apply_patch", "patch":patch("private/marker")
        }),
    )
    .await;
    let unchanged = fs::read(private.join("marker")).unwrap() == b"before\n";
    eprintln!("HOST_PRIVATE_MUTATION case=patch actual_builtin=true refused={} unchanged={unchanged} llm_calls=0", result.is_err());
    assert!(result.is_err() && unchanged);
}

#[tokio::test]
async fn host_private_mutation_rewind_rechecks_current_target_policy() {
    let root = TempDirGuard::new("private_mutation_rewind");
    let private = private_directory(&root.path);
    fs::write(private.join("marker"), b"before\n").unwrap();
    let public_state = test_state(root.path.clone());
    let applied = execute_builtin_skill(
        &public_state,
        "write_file",
        &json!({
            "path":"private/marker", "content":"after\n"
        }),
    )
    .await
    .unwrap();
    let applied: Value = serde_json::from_str(&applied).unwrap();
    let state = private_state(&root.path, &private);
    let result = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"rewind", "checkpoint_id":applied["checkpoint_id"]
        }),
    )
    .await;
    let unchanged = fs::read(private.join("marker")).unwrap() == b"after\n";
    eprintln!("HOST_PRIVATE_MUTATION case=rewind actual_builtin=true refused={} unchanged={unchanged} llm_calls=0", result.is_err());
    assert!(result.is_err() && unchanged);
}

#[tokio::test]
async fn host_private_mutation_refuses_private_checkpoint_staging() {
    let root = TempDirGuard::new("private_mutation_checkpoint");
    fs::write(root.path.join("public"), b"before\n").unwrap();
    let state_root = claw_core::workspace_state::workspace_state_root(&root.path);
    fs::create_dir_all(&state_root).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let state = private_state(&root.path, &state_root);
    let result = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path":"./public", "content":"after\n"
        }),
    )
    .await;
    let unchanged = fs::read(root.path.join("public")).unwrap() == b"before\n";
    let no_staging = fs::read_dir(&state_root).unwrap().next().is_none();
    eprintln!("HOST_PRIVATE_MUTATION case=checkpoint actual_builtin=true refused={} unchanged={unchanged} no_staging={no_staging} llm_calls=0", result.is_err());
    assert!(result.is_err() && unchanged && no_staging);
}

#[tokio::test]
async fn host_private_mutation_diff_refuses_newly_private_checkpoint() {
    let root = TempDirGuard::new("private_mutation_diff");
    fs::write(root.path.join("public"), b"before\n").unwrap();
    let public_state = test_state(root.path.clone());
    let applied = execute_builtin_skill(
        &public_state,
        "workspace_patch",
        &json!({
            "action":"apply_patch", "patch":patch("public")
        }),
    )
    .await
    .unwrap();
    let applied: Value = serde_json::from_str(&applied).unwrap();
    let state_root = claw_core::workspace_state::workspace_state_root(&root.path);
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let state = private_state(&root.path, &state_root);
    let result = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"diff", "checkpoint_id":applied["checkpoint_id"]
        }),
    )
    .await;
    eprintln!(
        "HOST_PRIVATE_MUTATION case=diff actual_builtin=true refused={} llm_calls=0",
        result.is_err()
    );
    assert!(
        result.is_err(),
        "checkpoint reads must use the current host policy"
    );
}
