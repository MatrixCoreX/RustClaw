use std::fs;

use serde_json::json;

use super::{
    init_git_fixture_repo, install_registry_from_toml, test_state, test_task, TempDirGuard,
};

#[tokio::test]
async fn task_scoped_worktree_reuses_root_across_multiple_skill_calls() {
    let root = TempDirGuard::new("task_scoped_worktree_multiple_calls");
    init_git_fixture_repo(root.path());
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_worktree", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let task = test_task(json!({"kind": "run_skill"}));
    let plan = crate::execution_isolation::plan_execution_isolation(
        root.path(),
        &task.task_id,
        claw_core::skill_registry::CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("task-scoped worktree plan");
    let runtime =
        crate::execution_isolation::create_execution_isolation(&plan, crate::now_ts_u64())
            .expect("create task-scoped worktree");
    state.skill_rt.workspace_root = runtime.plan.execution_root.clone();
    state.skill_rt.default_locator_search_dir = runtime.plan.execution_root.clone();

    for (path, content) in [("src/first.txt", "first\n"), ("src/second.txt", "second\n")] {
        let outcome = crate::run_skill_with_runner_outcome(
            &state,
            &task,
            "write_file",
            json!({"path": path, "content": content}),
        )
        .await
        .expect("write in reused task worktree");
        assert!(
            outcome
                .extra
                .as_ref()
                .and_then(|extra| extra.get("artifact_refs"))
                .is_none(),
            "skill call must not allocate a nested worktree"
        );
        assert_eq!(
            fs::read_to_string(runtime.plan.execution_root.join(path))
                .expect("read task-scoped worktree output"),
            content
        );
    }
    assert!(!root.path().join("src/first.txt").exists());
    assert!(!root.path().join("src/second.txt").exists());

    crate::execution_isolation::cleanup_execution_isolation(&runtime.plan)
        .expect("cleanup task-scoped worktree");
}
