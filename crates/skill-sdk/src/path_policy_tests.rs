use serde_json::json;

use super::{ExpectedPathKind, SkillPathPolicy};

fn admin_context() -> serde_json::Value {
    json!({
        "authority_scope": "unrestricted_admin",
        "permissions": {
            "unrestricted_admin": true,
            "allow_path_outside_workspace": true
        }
    })
}

#[test]
fn confined_policy_resolves_workspace_paths_and_rejects_escape() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(workspace.path().join("inside.txt"), "inside").expect("fixture");
    let outside = tempfile::NamedTempFile::new().expect("outside");
    let policy = SkillPathPolicy::new(workspace.path(), None).expect("policy");

    assert_eq!(
        policy
            .resolve_existing("inside.txt", ExpectedPathKind::File)
            .expect("inside"),
        workspace.path().join("inside.txt").canonicalize().unwrap()
    );
    assert_eq!(
        policy
            .resolve_existing(outside.path().to_str().unwrap(), ExpectedPathKind::File)
            .unwrap_err()
            .code,
        "path_outside_workspace"
    );
    assert_eq!(
        policy
            .resolve_create_target("../escape.txt")
            .unwrap_err()
            .code,
        "path_traversal_forbidden"
    );
}

#[test]
fn admin_policy_allows_external_paths_only_with_complete_verified_shape() {
    let workspace = tempfile::tempdir().expect("workspace");
    let outside = tempfile::NamedTempFile::new().expect("outside");
    let context = admin_context();
    let admin = SkillPathPolicy::new(workspace.path(), Some(&context)).expect("admin policy");
    assert!(admin.authority().is_unrestricted_admin());
    assert!(admin
        .resolve_existing(outside.path().to_str().unwrap(), ExpectedPathKind::File)
        .is_ok());

    let incomplete = json!({
        "authority_scope": "unrestricted_admin",
        "permissions": { "allow_path_outside_workspace": true }
    });
    let confined = SkillPathPolicy::new(workspace.path(), Some(&incomplete)).expect("policy");
    assert!(!confined.authority().is_unrestricted_admin());
    assert!(confined
        .resolve_existing(outside.path().to_str().unwrap(), ExpectedPathKind::File)
        .is_err());
}

#[test]
fn create_target_canonicalizes_existing_parent() {
    let workspace = tempfile::tempdir().expect("workspace");
    let policy = SkillPathPolicy::new(workspace.path(), None).expect("policy");
    let target = policy
        .resolve_create_target("new/deep/file.txt")
        .expect("create target");
    let canonical_workspace = workspace
        .path()
        .canonicalize()
        .expect("canonical workspace");
    assert_eq!(target, canonical_workspace.join("new/deep/file.txt"));
}

#[test]
fn create_target_preserves_existing_file_path() {
    let workspace = tempfile::tempdir().expect("workspace");
    let target = workspace.path().join("existing.zip");
    std::fs::write(&target, b"fixture").expect("existing target fixture");
    let policy = SkillPathPolicy::new(workspace.path(), None).expect("policy");

    let resolved = policy
        .resolve_create_target("existing.zip")
        .expect("resolve existing target");
    let canonical_target = target.canonicalize().expect("canonical target");

    assert_eq!(resolved, canonical_target);
}

#[cfg(unix)]
#[test]
fn confined_policy_rejects_symlink_escape_and_symlink_mutation_target() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("secret.txt"), "secret").expect("fixture");
    symlink(outside.path(), workspace.path().join("escape")).expect("symlink directory");
    symlink(
        outside.path().join("secret.txt"),
        workspace.path().join("target.txt"),
    )
    .expect("symlink file");
    let policy = SkillPathPolicy::new(workspace.path(), None).expect("policy");

    assert_eq!(
        policy
            .resolve_existing("escape/secret.txt", ExpectedPathKind::File)
            .unwrap_err()
            .code,
        "path_outside_workspace"
    );
    assert_eq!(
        policy.resolve_create_target("target.txt").unwrap_err().code,
        "path_target_symlink_forbidden"
    );
}
