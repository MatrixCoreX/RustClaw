use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use claw_core::skill_registry::CapabilityIsolationProfile;
use serde_json::{json, Value};

use super::execute_child_task_patch;
use crate::execution_isolation::{
    build_child_worktree_patch_artifact, create_execution_isolation, plan_execution_isolation,
};
use crate::ClaimedTask;

struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_parent_child_patch_{label}_{}_{}",
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

struct ChildPatchFixture {
    _repo: TempRepo,
    state: crate::AppState,
    parent: ClaimedTask,
    child_task_id: String,
    worktree_root: PathBuf,
    artifact_path: PathBuf,
    patch_ref: String,
}

impl ChildPatchFixture {
    fn new(label: &str) -> Self {
        let repo = TempRepo::new(label);
        let db_path = repo.path.join("tasks.sqlite");
        let mut state = file_backed_state_with_schema(&db_path);
        state.skill_rt.workspace_root = repo.path.clone();
        let parent_task_id = format!("parent-{label}");
        let child_task_id = format!("child-{label}");
        let plan = plan_execution_isolation(
            &repo.path,
            &child_task_id,
            CapabilityIsolationProfile::LocalWorktree,
        )
        .expect("plan child worktree");
        create_execution_isolation(&plan, 100).expect("create child worktree");
        fs::write(plan.execution_root.join("README.md"), "child change\n")
            .expect("modify tracked file");
        fs::create_dir_all(plan.execution_root.join("src")).expect("create source dir");
        fs::write(plan.execution_root.join("src/new.txt"), "child file\n")
            .expect("write new child file");
        let artifact =
            build_child_worktree_patch_artifact(&plan).expect("build child patch artifact");
        let artifact_path =
            PathBuf::from(artifact["artifact_path"].as_str().expect("artifact path"));
        let patch_ref = artifact["patch_ref"]
            .as_str()
            .expect("patch ref")
            .to_string();
        insert_task(
            &state,
            &parent_task_id,
            "running",
            &json!({"text": "parent"}),
            &json!({}),
        );
        insert_task(
            &state,
            &child_task_id,
            "succeeded",
            &child_payload(&parent_task_id, &child_task_id),
            &json!({
                "child_task_execution_scope": {
                    "patch_artifact": artifact
                },
                "task_journal": {
                    "summary": {
                        "coding_workflow": {
                            "verification_status": "verified",
                            "verification_command_count": 1,
                            "verification_commands": ["cargo test -p fixture"],
                            "failure_kind_count": 0,
                            "failure_kinds": [],
                            "changed_file_count": 2,
                            "changed_files": ["README.md", "src/new.txt"],
                            "validation_gate": {
                                "gate_status": "satisfied",
                                "can_report_fully_verified": true
                            }
                        }
                    }
                },
                "child_task_result": {
                    "verification_artifact": {
                        "schema_version": 1,
                        "kind": "child_task_verification",
                        "verification_status": "verified",
                        "verification_command_count": 1,
                        "verification_commands": ["cargo test -p fixture"],
                        "validation_gate": {
                            "gate_status": "satisfied",
                            "can_report_fully_verified": true
                        }
                    }
                }
            }),
        );
        let parent = ClaimedTask {
            claim_attempt: 0,
            task_id: parent_task_id,
            user_id: 42,
            chat_id: 7,
            user_key: Some("test-key".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: json!({"text": "parent"}).to_string(),
        };
        Self {
            _repo: repo,
            state,
            parent,
            child_task_id,
            worktree_root: plan.execution_root,
            artifact_path,
            patch_ref,
        }
    }

    fn run(&self, action: &str) -> Result<Value, String> {
        let args = json!({
            "action": action,
            "child_task_id": self.child_task_id,
            "patch_ref": self.patch_ref,
        });
        execute_child_task_patch(
            &self.state,
            Some(&self.parent),
            args.as_object().expect("args object"),
        )
        .and_then(|raw| serde_json::from_str(&raw).map_err(|err| err.to_string()))
    }
}

#[test]
fn parent_reviews_then_applies_child_patch_with_checkpoint_and_cleanup() {
    let fixture = ChildPatchFixture::new("apply");

    let review = fixture.run("review_child_patch").expect("review patch");
    assert_eq!(review["action"], "review_child_patch");
    assert_eq!(review["base_is_parent_ancestor"], true);
    assert_eq!(review["changed_file_count"], 2);
    assert_eq!(review["permission_profile"], "local_worktree");
    assert_eq!(review["allowed_capabilities"][0], "filesystem.write_text");
    assert_eq!(
        review["verification_artifact"]["verification_status"],
        "verified"
    );
    assert!(review["patch"]
        .as_str()
        .is_some_and(|patch| patch.contains("diff --git a/README.md b/README.md")));

    let applied = fixture.run("apply_child_patch").expect("apply patch");
    assert_eq!(applied["action"], "apply_child_patch");
    assert_eq!(applied["disposition"], "applied");
    assert_eq!(applied["cleanup_status"], "complete");
    assert_eq!(applied["workspace_patch"]["action"], "apply_patch");
    assert!(applied["workspace_patch"]["checkpoint_id"].is_string());
    assert_eq!(
        fs::read_to_string(fixture._repo.path.join("README.md")).expect("read primary"),
        "child change\n"
    );
    assert_eq!(
        fs::read_to_string(fixture._repo.path.join("src/new.txt")).expect("read new file"),
        "child file\n"
    );
    assert!(!fixture.worktree_root.exists());
    assert!(!fixture.artifact_path.exists());

    let repeated = fixture
        .run("apply_child_patch")
        .expect("repeat applied decision");
    assert_eq!(repeated["disposition"], "applied");
    assert_eq!(repeated["cleanup_status"], "complete");
    let child = stored_result_json(&fixture.state, &fixture.child_task_id);
    assert_eq!(
        child["child_task_execution_scope"]["patch_disposition"]["disposition"],
        "applied"
    );
    let parent = stored_result_json(&fixture.state, &fixture.parent.task_id);
    assert_eq!(
        parent["child_patch_dispositions"][0]["child_task_id"],
        fixture.child_task_id
    );
}

#[test]
fn parent_rejects_child_patch_without_mutating_primary_workspace() {
    let fixture = ChildPatchFixture::new("reject");

    let rejected = fixture.run("reject_child_patch").expect("reject patch");

    assert_eq!(rejected["action"], "reject_child_patch");
    assert_eq!(rejected["disposition"], "rejected");
    assert_eq!(rejected["cleanup_status"], "complete");
    assert_eq!(
        fs::read_to_string(fixture._repo.path.join("README.md")).expect("read primary"),
        "fixture\n"
    );
    assert!(!fixture._repo.path.join("src/new.txt").exists());
    assert!(!fixture.worktree_root.exists());
    assert!(!fixture.artifact_path.exists());
}

#[test]
fn parent_dirty_change_blocks_child_patch_and_preserves_review_artifacts() {
    let fixture = ChildPatchFixture::new("conflict");
    fs::write(fixture._repo.path.join("README.md"), "parent change\n")
        .expect("write conflicting parent change");

    let error = fixture
        .run("apply_child_patch")
        .expect_err("dirty parent file must block patch");

    assert!(error.contains("patch_precondition_failed"));
    assert_eq!(
        fs::read_to_string(fixture._repo.path.join("README.md")).expect("read primary"),
        "parent change\n"
    );
    assert!(fixture.worktree_root.exists());
    assert!(fixture.artifact_path.exists());
    let child = stored_result_json(&fixture.state, &fixture.child_task_id);
    assert!(child["child_task_execution_scope"]
        .get("patch_disposition")
        .is_none());
}

#[test]
fn persisted_path_ownership_blocks_out_of_scope_child_patch() {
    let fixture = ChildPatchFixture::new("ownership");
    let db = fixture.state.core.db.get().expect("get db");
    db.execute(
        "UPDATE child_task_graph_nodes
         SET owned_paths_json = '[\"src\"]'
         WHERE child_task_id = ?1",
        rusqlite::params![fixture.child_task_id],
    )
    .expect("narrow child ownership");
    drop(db);

    let error = fixture
        .run("review_child_patch")
        .expect_err("README patch must exceed src ownership");
    assert!(error.contains("child_patch_path_ownership_mismatch"));
    assert!(fixture.worktree_root.exists());
    assert!(fixture.artifact_path.exists());
}

#[test]
fn overlapping_child_patches_require_parent_resolution() {
    let fixture = ChildPatchFixture::new("overlap");
    let second_child_task_id = "child-overlap-second";
    let second_plan = plan_execution_isolation(
        &fixture._repo.path,
        second_child_task_id,
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("plan second child worktree");
    create_execution_isolation(&second_plan, 101).expect("create second child worktree");
    fs::write(
        second_plan.execution_root.join("README.md"),
        "second child change\n",
    )
    .expect("modify overlapping file");
    let second_artifact =
        build_child_worktree_patch_artifact(&second_plan).expect("build second patch artifact");
    let second_artifact_path = PathBuf::from(
        second_artifact["artifact_path"]
            .as_str()
            .expect("second artifact path"),
    );
    let second_patch_ref = second_artifact["patch_ref"]
        .as_str()
        .expect("second patch ref");
    insert_task(
        &fixture.state,
        second_child_task_id,
        "succeeded",
        &child_payload(&fixture.parent.task_id, second_child_task_id),
        &json!({
            "child_task_execution_scope": {
                "patch_artifact": second_artifact
            }
        }),
    );

    fixture
        .run("apply_child_patch")
        .expect("apply first child patch");
    let second_args = json!({
        "action": "apply_child_patch",
        "child_task_id": second_child_task_id,
        "patch_ref": second_patch_ref,
    });
    let error = execute_child_task_patch(
        &fixture.state,
        Some(&fixture.parent),
        second_args.as_object().expect("second args"),
    )
    .expect_err("overlapping child patch must fail precondition validation");
    let parsed =
        crate::skills::parse_structured_skill_error(&error).expect("structured conflict error");

    assert_eq!(parsed.error_kind, "patch_precondition_failed");
    assert!(second_plan.execution_root.exists());
    assert!(second_artifact_path.exists());

    let reject_args = json!({
        "action": "reject_child_patch",
        "child_task_id": second_child_task_id,
        "patch_ref": second_patch_ref,
    });
    execute_child_task_patch(
        &fixture.state,
        Some(&fixture.parent),
        reject_args.as_object().expect("reject args"),
    )
    .expect("reject conflicting child patch");
    assert!(!second_plan.execution_root.exists());
    assert!(!second_artifact_path.exists());
}

#[test]
fn writer_tester_reviewer_worktree_flow_applies_verified_patch_once() {
    let fixture = ChildPatchFixture::new("team-flow");
    let tester_task_id = "child-team-flow-tester";
    let explorer_task_ids = ["child-team-flow-explorer-a", "child-team-flow-explorer-b"];
    let tester_plan = plan_execution_isolation(
        &fixture._repo.path,
        tester_task_id,
        CapabilityIsolationProfile::LocalWorktree,
    )
    .expect("plan tester worktree");
    create_execution_isolation(&tester_plan, 102).expect("create tester worktree");
    let patch_path = fixture
        .artifact_path
        .to_str()
        .expect("UTF-8 patch artifact path");
    assert!(git_status(
        &tester_plan.execution_root,
        &["apply", "--check", patch_path]
    )
    .success());
    assert!(git_status(&tester_plan.execution_root, &["apply", patch_path]).success());
    assert!(git_status(&tester_plan.execution_root, &["diff", "--check"]).success());
    assert_eq!(
        fs::read_to_string(tester_plan.execution_root.join("README.md"))
            .expect("read tester README"),
        "child change\n"
    );
    assert_eq!(
        fs::read_to_string(tester_plan.execution_root.join("src/new.txt"))
            .expect("read tester new file"),
        "child file\n"
    );

    let tester_payload = child_payload_with_contract(
        &fixture.parent.task_id,
        tester_task_id,
        "tester",
        "local_worktree",
        true,
        &["filesystem.read_text", "terminal.run_command"],
    );
    insert_task(
        &fixture.state,
        tester_task_id,
        "succeeded",
        &tester_payload,
        &json!({
            "task_journal": {
                "summary": {
                    "coding_workflow": {
                        "verification_status": "verified",
                        "verification_command_count": 2,
                        "verification_commands": ["git apply --check", "git diff --check"],
                        "failure_kind_count": 0,
                        "failure_kinds": [],
                        "changed_file_count": 2,
                        "changed_files": ["README.md", "src/new.txt"],
                        "validation_gate": {
                            "gate_status": "satisfied",
                            "can_report_fully_verified": true
                        }
                    }
                }
            }
        }),
    );
    for explorer_task_id in explorer_task_ids {
        let payload = child_payload_with_contract(
            &fixture.parent.task_id,
            explorer_task_id,
            "explorer",
            "read_only",
            false,
            &["filesystem.read_text"],
        );
        insert_task(
            &fixture.state,
            explorer_task_id,
            "succeeded",
            &payload,
            &json!({"status_code": "inspection_complete"}),
        );
        assert!(
            crate::repo::child_tasks::record_child_task_terminal_projection(
                &fixture.state,
                explorer_task_id,
                &payload,
            )
            .expect("record explorer projection")
        );
    }
    {
        let db = fixture.state.core.db.get().expect("get db");
        db.execute(
            "UPDATE tasks SET result_json = ?2 WHERE task_id = ?1",
            rusqlite::params![
                fixture.parent.task_id,
                json!({
                    "child_task_ids": [
                        fixture.child_task_id,
                        tester_task_id,
                        explorer_task_ids[0],
                        explorer_task_ids[1]
                    ]
                })
                .to_string()
            ],
        )
        .expect("record parent child ids");
    }
    assert!(
        crate::repo::child_tasks::record_child_task_terminal_projection(
            &fixture.state,
            &fixture.child_task_id,
            &child_payload(&fixture.parent.task_id, &fixture.child_task_id),
        )
        .expect("record writer projection")
    );
    assert!(
        crate::repo::child_tasks::record_child_task_terminal_projection(
            &fixture.state,
            tester_task_id,
            &tester_payload,
        )
        .expect("record tester projection")
    );
    let merge = crate::repo::child_tasks::refresh_parent_child_task_merge(
        &fixture.state,
        &fixture.parent.task_id,
    )
    .expect("refresh parent merge")
    .expect("parent merge");
    assert_eq!(merge["parent_continuation"]["status"], "ready");
    assert_eq!(merge["merge"]["completed_count"], 4);
    assert_eq!(merge["merge"]["required_failed_count"], 0);

    let review = fixture
        .run("review_child_patch")
        .expect("review writer patch");
    assert_eq!(
        review["verification_artifact"]["verification_status"],
        "verified"
    );
    let applied = fixture
        .run("apply_child_patch")
        .expect("apply writer patch");
    assert_eq!(applied["disposition"], "applied");
    assert_eq!(
        fs::read_to_string(fixture._repo.path.join("README.md")).expect("read primary README"),
        "child change\n"
    );
    let repeated = fixture
        .run("apply_child_patch")
        .expect("repeat parent decision");
    assert_eq!(repeated["disposition"], "applied");

    crate::execution_isolation::cleanup_execution_isolation(&tester_plan)
        .expect("cleanup tester worktree");
}

#[test]
fn unrelated_parent_cannot_review_or_decide_child_patch() {
    let fixture = ChildPatchFixture::new("ownership");
    let mut unrelated_parent = fixture.parent.clone();
    unrelated_parent.task_id = "different-parent".to_string();
    let args = json!({
        "action": "review_child_patch",
        "child_task_id": fixture.child_task_id,
    });

    let error = execute_child_task_patch(
        &fixture.state,
        Some(&unrelated_parent),
        args.as_object().expect("args object"),
    )
    .expect_err("unrelated parent must be denied");

    assert!(error.contains("child_patch_parent_mismatch"));
    assert!(fixture.worktree_root.exists());
    assert!(fixture.artifact_path.exists());
}

fn child_payload(parent_task_id: &str, child_task_id: &str) -> Value {
    child_payload_with_contract(
        parent_task_id,
        child_task_id,
        "writer",
        "local_worktree",
        true,
        &["filesystem.write_text"],
    )
}

fn child_payload_with_contract(
    parent_task_id: &str,
    child_task_id: &str,
    role: &str,
    permission_profile: &str,
    required: bool,
    allowed_capabilities: &[&str],
) -> Value {
    json!({
        "text": "child",
        "task_role": "subagent_child",
        "parent_task_id": parent_task_id,
        "child_task_id": child_task_id,
        "child_task_contract": {
            "schema_version": 1,
            "parent_task_id": parent_task_id,
            "child_task_id": child_task_id,
            "role": role,
            "permission_profile": permission_profile,
            "scope": {
                "objective": "bounded child objective",
                "allowed_capabilities": allowed_capabilities
            },
            "required": required,
            "merge_policy": "structured_findings"
        }
    })
}

fn insert_task(
    state: &crate::AppState,
    task_id: &str,
    status: &str,
    payload: &Value,
    result: &Value,
) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, ?3, ?4, NULL, '1', '1')",
        rusqlite::params![task_id, payload.to_string(), status, result.to_string()],
    )
    .expect("insert task");
    if payload.get("task_role").and_then(Value::as_str) == Some("subagent_child") {
        let contract = payload
            .get("child_task_contract")
            .and_then(Value::as_object)
            .expect("child contract");
        let parent_task_id = contract["parent_task_id"].as_str().expect("parent task id");
        let role = contract["role"].as_str().expect("role");
        let permission_profile = contract["permission_profile"]
            .as_str()
            .expect("permission profile");
        let required = contract["required"].as_bool().expect("required");
        db.execute(
            "INSERT INTO child_task_graphs (
                parent_task_id, schema_version, status, max_parallel, created_at, updated_at
             ) VALUES (?1, 1, 'active', 4, '1', '1')
             ON CONFLICT(parent_task_id) DO NOTHING",
            rusqlite::params![parent_task_id],
        )
        .expect("insert graph");
        db.execute(
            "INSERT INTO child_task_graph_nodes (
                parent_task_id, child_task_id, role, required, readiness,
                permission_profile, merge_policy, owned_paths_json, budget_json,
                model_policy_json, tool_policy_json, result_contract_json,
                steering_version, steering_json, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, 'structured_findings', '[\".\"]',
                '{}', '{}', '{}', '{}', 0, '{}', '1', '1'
             )",
            rusqlite::params![
                parent_task_id,
                task_id,
                role,
                required,
                status,
                permission_profile
            ],
        )
        .expect("insert graph node");
    }
}

fn stored_result_json(state: &crate::AppState, task_id: &str) -> Value {
    let db = state.core.db.get().expect("get db");
    let raw: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("select result_json");
    serde_json::from_str(&raw).expect("parse result_json")
}

fn file_backed_state_with_schema(db_path: &Path) -> crate::AppState {
    let manager = r2d2_sqlite::SqliteConnectionManager::file(db_path).with_init(
        |connection: &mut rusqlite::Connection| {
            connection.busy_timeout(Duration::from_millis(5_000))?;
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "NORMAL")?;
            connection.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        },
    );
    let pool = r2d2::Pool::builder()
        .max_size(2)
        .build(manager)
        .expect("build db pool");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.db = pool;
    state.worker.database_sqlite_path = db_path.to_path_buf();
    state.with_seeded_db_schema()
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
