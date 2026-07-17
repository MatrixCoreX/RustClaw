use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use claw_core::skill_registry::CapabilityIsolationProfile;

use crate::execution_isolation::{
    build_child_worktree_patch_artifact, cleanup_execution_isolation, create_execution_isolation,
    plan_execution_isolation,
};

struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_patch_artifact_{label}_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).expect("create temp repo");
        init_git_repo(&path);
        Self { path }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn child_worktree_patch_contains_tracked_and_untracked_changes_without_staging() {
    let repo = TempRepo::new("complete");
    let plan = plan_execution_isolation(
        &repo.path,
        "child-patch-complete",
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("worktree plan");
    create_execution_isolation(&plan, 100).expect("create worktree");
    fs::write(plan.execution_root.join("README.md"), "changed\n").expect("change tracked file");
    fs::create_dir_all(plan.execution_root.join("src")).expect("create src");
    fs::write(plan.execution_root.join("src/new.txt"), "new\n").expect("write untracked file");

    let artifact = build_child_worktree_patch_artifact(&plan).expect("build child patch artifact");

    assert_eq!(artifact["kind"], "child_worktree_patch");
    assert_eq!(artifact["status"], "ready");
    assert_eq!(artifact["changed_file_count"], 2);
    assert_eq!(
        artifact["changed_files"],
        serde_json::json!(["README.md", "src/new.txt"])
    );
    assert_eq!(artifact["apply_owner"], "parent_agent");
    assert_eq!(artifact["apply_policy"], "parent_review_required");
    let patch_path = artifact["artifact_path"]
        .as_str()
        .expect("patch artifact path");
    let patch = fs::read_to_string(patch_path).expect("read patch");
    assert!(patch.contains("diff --git a/README.md b/README.md"));
    assert!(patch.contains("diff --git a/src/new.txt b/src/new.txt"));
    assert!(!patch.contains(".rustclaw-isolation.json"));
    assert!(git_status(&plan.execution_root, &["diff", "--cached", "--quiet"]).success());
    assert!(git_status(&repo.path, &["apply", "--check", patch_path]).success());

    cleanup_execution_isolation(&plan).expect("cleanup worktree");
}

#[test]
fn unchanged_child_worktree_returns_empty_review_artifact() {
    let repo = TempRepo::new("empty");
    let plan = plan_execution_isolation(
        &repo.path,
        "child-patch-empty",
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("worktree plan");
    create_execution_isolation(&plan, 100).expect("create worktree");

    let artifact =
        build_child_worktree_patch_artifact(&plan).expect("build empty child patch artifact");

    assert_eq!(artifact["status"], "empty");
    assert_eq!(artifact["changed_file_count"], 0);
    assert!(artifact["patch_ref"].is_null());
    assert!(artifact["artifact_path"].is_null());

    cleanup_execution_isolation(&plan).expect("cleanup worktree");
}

fn init_git_repo(path: &Path) {
    for args in [
        ["init", "--quiet"].as_slice(),
        ["config", "user.email", "rustclaw-test@example.invalid"].as_slice(),
        ["config", "user.name", "RustClaw Test"].as_slice(),
    ] {
        assert!(git_status(path, args).success());
    }
    fs::write(path.join("README.md"), "fixture\n").expect("write fixture");
    assert!(git_status(path, &["add", "README.md"]).success());
    assert!(git_status(path, &["commit", "--quiet", "-m", "fixture"]).success());
}

fn git_status(path: &Path, args: &[&str]) -> std::process::ExitStatus {
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .expect("run git")
}
