use super::{execute_local_write, is_local_write_action};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestRepository {
    root: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-git-local-write-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create repository");
        git(&root, &["init", "--quiet"]);
        git(&root, &["config", "user.name", "Agent Runtime Test"]);
        git(
            &root,
            &["config", "user.email", "agent-runtime-test@example.invalid"],
        );
        std::fs::write(root.join("tracked.txt"), "initial\n").expect("fixture");
        git(&root, &["add", "--", "tracked.txt"]);
        git(&root, &["commit", "--quiet", "-m", "initial"]);
        Self { root }
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn git(root: &Path, argv: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(argv)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {argv:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn action_set_contains_local_mutations_but_not_remote_publish() {
    for action in ["stage", "commit", "create_branch", "checkout_branch"] {
        assert!(is_local_write_action(action));
    }
    for action in ["status", "push", "fetch", "pull"] {
        assert!(!is_local_write_action(action));
    }
}

#[test]
fn stage_and_commit_support_spaces_and_unicode_paths() {
    let repo = TestRepository::new();
    let path = "docs/说明 file.txt";
    std::fs::create_dir(repo.root.join("docs")).expect("docs");
    std::fs::write(repo.root.join(path), "content\n").expect("unicode fixture");
    git(&repo.root, &["config", "commit.gpgSign", "true"]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let hook = repo.root.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\nexit 1\n").expect("hook fixture");
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .expect("hook executable");
    }

    let (_, staged) =
        execute_local_write(&repo.root, &args(json!({"paths": [path]})), "stage").expect("stage");
    assert_eq!(staged["staged_paths"], json!([path]));
    assert_eq!(staged["remote_mutation"], false);

    let (_, committed) = execute_local_write(
        &repo.root,
        &args(json!({"message": "add Unicode path"})),
        "commit",
    )
    .expect("commit");
    assert!(committed["commit_hash"]
        .as_str()
        .is_some_and(|hash| hash.len() == 40));
    assert_eq!(committed["staged_paths"], json!([]));
    assert_eq!(committed["hooks_enabled"], false);
    assert_eq!(committed["signing_enabled"], false);
    assert!(!repo.root.join("hook-ran").exists());
}

#[test]
fn stage_requires_explicit_nonempty_paths_and_commit_requires_stage() {
    let repo = TestRepository::new();
    for value in [json!({}), json!({"paths": []}), json!({"paths": ["."]})] {
        assert!(execute_local_write(&repo.root, &args(value), "stage").is_err());
    }
    let commit = execute_local_write(
        &repo.root,
        &args(json!({"message": "must not commit"})),
        "commit",
    )
    .expect_err("empty stage");
    assert_eq!(commit.code, "git_stage_empty");
    let unexpected = execute_local_write(
        &repo.root,
        &args(json!({"paths": ["tracked.txt"], "message": "not valid for stage"})),
        "stage",
    )
    .expect_err("closed action shape");
    assert_eq!(unexpected.code, "git_unexpected_arg");
}

#[test]
fn branch_creation_and_checkout_refuse_dirty_overwrite() {
    let repo = TestRepository::new();
    execute_local_write(
        &repo.root,
        &args(json!({"branch_name": "feature/local"})),
        "create_branch",
    )
    .expect("create branch");
    std::fs::write(repo.root.join("tracked.txt"), "dirty\n").expect("dirty fixture");

    let dirty = execute_local_write(
        &repo.root,
        &args(json!({"branch_name": "feature/local"})),
        "checkout_branch",
    )
    .expect_err("dirty checkout");
    assert_eq!(dirty.code, "git_checkout_dirty_worktree");

    git(&repo.root, &["restore", "--", "tracked.txt"]);
    let (_, checked_out) = execute_local_write(
        &repo.root,
        &args(json!({"branch_name": "feature/local"})),
        "checkout_branch",
    )
    .expect("clean checkout");
    assert_eq!(checked_out["branch"], "feature/local");
    assert_eq!(checked_out["clean"], true);
}

#[test]
fn branch_creation_handles_detached_head_and_rejects_existing_branch() {
    let repo = TestRepository::new();
    git(&repo.root, &["checkout", "--quiet", "--detach", "HEAD"]);

    execute_local_write(
        &repo.root,
        &args(json!({"branch_name": "feature/detached"})),
        "create_branch",
    )
    .expect("create branch from detached head");
    let conflict = execute_local_write(
        &repo.root,
        &args(json!({"branch_name": "feature/detached"})),
        "create_branch",
    )
    .expect_err("existing branch conflict");
    assert_eq!(conflict.code, "git_command_failed");

    let (_, checked_out) = execute_local_write(
        &repo.root,
        &args(json!({"branch_name": "feature/detached"})),
        "checkout_branch",
    )
    .expect("checkout from detached head");
    assert_eq!(checked_out["branch"], "feature/detached");
}
