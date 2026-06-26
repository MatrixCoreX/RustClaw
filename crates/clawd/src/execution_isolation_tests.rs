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
fn isolation_profile_from_token_accepts_only_machine_tokens() {
    assert_eq!(
        isolation_profile_from_token("local_temp_workspace"),
        Some(CapabilityIsolationProfile::LocalTempWorkspace)
    );
    assert_eq!(isolation_profile_from_token("Local Temp Workspace"), None);
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
