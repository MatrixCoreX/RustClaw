use std::path::{Path, PathBuf};

use serde_json::json;

use super::ChildTaskExecutionScope;

struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_child_scope_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("create temp repo");
        init_git_repo(&path);
        Self { path }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn child_task(permission_profile: &str) -> (crate::ClaimedTask, serde_json::Value) {
    let payload = json!({
        "text": "bounded child objective",
        "task_role": "subagent_child",
        "parent_task_id": "parent-task",
        "child_task_id": "child-task",
        "child_task_contract": {
            "schema_version": 1,
            "parent_task_id": "parent-task",
            "child_task_id": "child-task",
            "permission_profile": permission_profile,
            "scope": {
                "objective": "bounded child objective",
                "allowed_capabilities": ["filesystem.read_text_range"],
            },
        },
    });
    (
        crate::ClaimedTask {
            claim_attempt: 0,
            task_id: "child-task".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: payload.to_string(),
        },
        payload,
    )
}

#[test]
fn local_worktree_child_binds_and_reuses_task_scoped_workspace() {
    let repo = TempRepo::new();
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = repo.path.clone();
    state.skill_rt.default_locator_search_dir = repo.path.clone();
    let (task, payload) = child_task("local_worktree");

    let mut first =
        ChildTaskExecutionScope::prepare(&state, &task, &payload).expect("prepare first scope");
    let plan = first.plan().expect("worktree plan").clone();
    assert_ne!(first.state(&state).skill_rt.workspace_root, repo.path);
    assert_eq!(
        first.state(&state).skill_rt.workspace_root,
        plan.execution_root
    );
    assert_eq!(
        first.projection(&state).expect("projection")["allocation_reused"],
        false
    );
    first.retain_for_parent_decision();

    let mut second =
        ChildTaskExecutionScope::prepare(&state, &task, &payload).expect("reuse child scope");
    assert_eq!(
        second.state(&state).skill_rt.workspace_root,
        plan.execution_root
    );
    assert_eq!(
        second.projection(&state).expect("projection")["allocation_reused"],
        true
    );
    second.retain_for_parent_decision();

    crate::execution_isolation::cleanup_execution_isolation(&plan).expect("cleanup child worktree");
}

#[test]
fn read_only_child_keeps_primary_root_without_allocation() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let (task, payload) = child_task("read_only");

    let scope = ChildTaskExecutionScope::prepare(&state, &task, &payload)
        .expect("prepare read-only child scope");
    let projection = scope.projection(&state).expect("projection");

    assert_eq!(
        scope.state(&state).skill_rt.workspace_root,
        state.skill_rt.workspace_root
    );
    assert!(scope.plan().is_none());
    assert_eq!(
        projection["workspace_binding"],
        "primary_workspace_read_only"
    );
    assert_eq!(projection["artifact_refs"], json!([]));
}

#[test]
fn dropped_unretained_child_scope_cleans_worktree_and_patch_artifact() {
    let repo = TempRepo::new();
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = repo.path.clone();
    state.skill_rt.default_locator_search_dir = repo.path.clone();
    let (task, payload) = child_task("local_worktree");

    let (worktree_root, artifact_path) = {
        let scope =
            ChildTaskExecutionScope::prepare(&state, &task, &payload).expect("prepare child scope");
        let plan = scope.plan().expect("worktree plan").clone();
        std::fs::write(plan.execution_root.join("README.md"), "partial\n")
            .expect("write partial child change");
        let artifact = crate::execution_isolation::build_child_worktree_patch_artifact(&plan)
            .expect("build partial patch");
        (
            plan.execution_root,
            PathBuf::from(
                artifact["artifact_path"]
                    .as_str()
                    .expect("partial artifact path"),
            ),
        )
    };

    assert!(!worktree_root.exists());
    assert!(!artifact_path.exists());
}

#[test]
fn unsupported_child_profile_fails_closed() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let (task, payload) = child_task("danger_full");

    let error = ChildTaskExecutionScope::prepare(&state, &task, &payload)
        .err()
        .expect("unsupported child profile must fail");
    assert_eq!(error.to_string(), "child_permission_profile_unsupported");
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
    std::fs::write(path.join("README.md"), "fixture\n").expect("write fixture");
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
