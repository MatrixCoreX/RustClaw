use super::*;

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_execution_isolation_{name}_{}_{}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp root");
        Self { path }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn isolation_plan_reuses_read_only_workspace_without_cleanup() {
    let root = TempRoot::new("read_only");

    let plan = plan_execution_isolation(
        &root.path,
        "task-read-only",
        CapabilityIsolationProfile::ReadOnly,
    )
    .expect("read-only isolation plan");
    let artifact = execution_isolation_artifact_ref(&plan);

    assert_eq!(plan.profile, "read_only");
    assert_eq!(plan.execution_root, root.path);
    assert!(plan.read_only);
    assert!(!plan.requires_cleanup);
    assert_eq!(artifact["kind"], "execution_isolation_workspace");
    assert_eq!(artifact["read_only"], true);
}

#[test]
fn local_temp_isolation_creates_marker_and_artifact_ref_then_cleans_up() {
    let root = TempRoot::new("temp");
    let plan = plan_execution_isolation(
        &root.path,
        "task/temp:unsafe",
        CapabilityIsolationProfile::LocalTempWorkspace,
    )
    .expect("temp isolation plan");

    let runtime = create_execution_isolation(&plan, 100).expect("create temp isolation");

    assert!(runtime.plan.execution_root.exists());
    assert!(runtime.plan.execution_root.join(MARKER_FILE).exists());
    assert_eq!(
        runtime.artifact_refs[0]["profile"],
        CapabilityIsolationProfile::LocalTempWorkspace.as_token()
    );
    assert_eq!(
        runtime.artifact_refs[0]["artifact_path"],
        runtime.plan.execution_root.display().to_string()
    );
    assert_eq!(runtime.artifact_refs[0]["requires_cleanup"], true);

    cleanup_execution_isolation(&runtime.plan).expect("cleanup temp isolation");
    assert!(!runtime.plan.execution_root.exists());
}

#[test]
fn abandoned_temp_cleanup_removes_only_stale_marked_dirs() {
    let root = TempRoot::new("cleanup");
    let stale = plan_execution_isolation(
        &root.path,
        "task-stale",
        CapabilityIsolationProfile::LocalTempWorkspace,
    )
    .expect("stale plan");
    let fresh = plan_execution_isolation(
        &root.path,
        "task-fresh",
        CapabilityIsolationProfile::LocalTempWorkspace,
    )
    .expect("fresh plan");
    create_execution_isolation(&stale, 10).expect("create stale");
    create_execution_isolation(&fresh, 95).expect("create fresh");

    let report = cleanup_abandoned_isolation_workspaces(&root.path, 100, 60);

    assert_eq!(report.removed, 1);
    assert!(report.errors.is_empty());
    assert!(!stale.execution_root.exists());
    assert!(fresh.execution_root.exists());
}

#[test]
fn abandoned_worktree_cleanup_removes_matching_patch_artifact() {
    let root = TempRoot::new("worktree_cleanup");
    init_git_repo(&root.path);
    let stale = plan_execution_isolation(
        &root.path,
        "task-stale-writer",
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("stale worktree plan");
    create_execution_isolation(&stale, 10).expect("create stale worktree");
    fs::write(
        stale.execution_root.join("README.md"),
        "stale child change\n",
    )
    .expect("modify stale worktree");
    let artifact =
        build_child_worktree_patch_artifact(&stale).expect("build stale child patch artifact");
    let artifact_path = PathBuf::from(
        artifact["artifact_path"]
            .as_str()
            .expect("stale artifact path"),
    );

    let report = cleanup_abandoned_isolation_workspaces(&root.path, 100, 60);

    assert_eq!(report.removed, 1);
    assert!(report.errors.is_empty());
    assert!(!stale.execution_root.exists());
    assert!(!artifact_path.exists());
}

#[test]
fn abandoned_cleanup_removes_stale_orphan_patch_artifact() {
    let root = TempRoot::new("orphan_patch_cleanup");
    let artifact_root = isolation_artifact_dir(&root.path);
    fs::create_dir_all(&artifact_root).expect("create artifact root");
    let stale = artifact_root.join("task-stale.patch");
    fs::write(&stale, "stale patch\n").expect("write stale patch");
    let modified_at = fs::metadata(&stale)
        .expect("stale patch metadata")
        .modified()
        .expect("stale patch modified time")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("stale patch unix time")
        .as_secs();

    let report = cleanup_abandoned_isolation_workspaces(&root.path, modified_at + 61, 60);

    assert_eq!(report.removed, 0);
    assert_eq!(report.artifacts_removed, 1);
    assert!(report.errors.is_empty());
    assert!(!stale.exists());
}

#[test]
fn abandoned_cleanup_retains_fresh_orphan_patch_artifact() {
    let root = TempRoot::new("fresh_orphan_patch_cleanup");
    let artifact_root = isolation_artifact_dir(&root.path);
    fs::create_dir_all(&artifact_root).expect("create artifact root");
    let fresh = artifact_root.join("task-fresh.patch");
    fs::write(&fresh, "fresh patch\n").expect("write fresh patch");
    let modified_at = fs::metadata(&fresh)
        .expect("fresh patch metadata")
        .modified()
        .expect("fresh patch modified time")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("fresh patch unix time")
        .as_secs();

    let report = cleanup_abandoned_isolation_workspaces(&root.path, modified_at + 59, 60);

    assert_eq!(report.artifacts_removed, 0);
    assert!(report.errors.is_empty());
    assert!(fresh.exists());
}

#[test]
fn local_worktree_plan_uses_isolated_cleanup_ref() {
    let root = TempRoot::new("worktree_plan");

    let plan = plan_execution_isolation(
        &root.path,
        "task-worktree",
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("worktree isolation plan");

    assert_eq!(plan.profile, "local_worktree");
    assert_eq!(plan.creation_kind, "create_local_git_worktree");
    assert!(plan.requires_cleanup);
    assert_eq!(
        plan.cleanup_ref.as_deref(),
        Some("isolation:worktrees:task-worktree")
    );
    assert!(plan.execution_root.ends_with("task-worktree"));
}

#[test]
fn local_worktree_allocation_reuses_matching_task_scope() {
    let root = TempRoot::new("worktree_reuse");
    init_git_repo(&root.path);
    let plan = plan_execution_isolation(
        &root.path,
        "task-worktree-reuse",
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("worktree isolation plan");

    let created =
        create_or_reuse_execution_isolation(&plan, 100).expect("create child worktree scope");
    let reused =
        create_or_reuse_execution_isolation(&plan, 101).expect("reuse child worktree scope");

    assert!(!created.reused);
    assert!(reused.reused);
    assert_eq!(
        reused.artifact_refs[0]["allocation_state"],
        serde_json::json!("reused")
    );
    assert_eq!(
        execution_isolation_root_profile(&plan.execution_root).as_deref(),
        Some("local_worktree")
    );
    cleanup_execution_isolation(&plan).expect("cleanup reused worktree");
}

#[test]
fn existing_isolation_with_mismatched_marker_fails_closed() {
    let root = TempRoot::new("worktree_marker_mismatch");
    init_git_repo(&root.path);
    let plan = plan_execution_isolation(
        &root.path,
        "task-worktree-mismatch",
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("worktree isolation plan");
    create_execution_isolation(&plan, 100).expect("create child worktree scope");
    let marker_path = plan.execution_root.join(MARKER_FILE);
    let mut marker: Value = serde_json::from_slice(&fs::read(&marker_path).expect("read marker"))
        .expect("parse marker");
    marker["task_key"] = serde_json::json!("another-task");
    fs::write(
        &marker_path,
        serde_json::to_vec_pretty(&marker).expect("serialize marker"),
    )
    .expect("replace marker");

    let error = create_or_reuse_execution_isolation(&plan, 101)
        .expect_err("mismatched worktree marker must not be reused");
    assert!(error
        .to_string()
        .contains("existing_isolation_contract_mismatch:task_key"));
    cleanup_execution_isolation(&plan).expect("cleanup mismatched worktree");
}

#[test]
fn isolation_profile_from_token_accepts_only_machine_tokens() {
    assert_eq!(
        isolation_profile_from_token("local_temp_workspace"),
        Some(CapabilityIsolationProfile::LocalTempWorkspace)
    );
    assert_eq!(isolation_profile_from_token("Local Temp Workspace"), None);
}

fn init_git_repo(path: &Path) {
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.email", "rustclaw-test@example.invalid"],
        vec!["config", "user.name", "RustClaw Test"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("run git setup");
        assert!(status.success());
    }
    fs::write(path.join("README.md"), "fixture\n").expect("write fixture");
    for args in [
        vec!["add", "README.md"],
        vec!["commit", "--quiet", "-m", "fixture"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("run git commit");
        assert!(status.success());
    }
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
