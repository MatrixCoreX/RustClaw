use std::sync::Arc;

use claw_core::config::{ToolApprovalPolicy, ToolSandboxMode, ToolsConfig};
use rusqlite::params;
use serde_json::{json, Value};

use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
use crate::PlanStep;

fn confirmation_step() -> PlanStep {
    PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({"command": "pwd"}),
        depends_on: Vec::new(),
        why: String::new(),
    }
}

fn scoped_confirmation_step(path: &str) -> PlanStep {
    PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: json!({
            "action": "write_text",
            "path": path,
            "content": "updated"
        }),
        depends_on: Vec::new(),
        why: String::new(),
    }
}

#[test]
fn verifier_consumes_exact_approval_once() {
    let state = super::tests::test_state();
    let task = super::tests::test_task();
    let step = confirmation_step();
    let binding = crate::approval_grant::binding_for_confirmation_steps(
        &state,
        std::slice::from_ref(&step),
        &[step.step_id.clone()],
    )
    .expect("approval binding");
    let approval_request = json!({
        "schema_version": 1,
        "request_id": "approval-verifier-1",
        "task_id": task.task_id,
        "status": "approved",
        "action_fingerprint": binding.action_fingerprint,
        "arguments_hash": binding.arguments_hash,
        "action_count": binding.action_count,
        "targets": binding.targets,
        "issued_at": crate::now_ts_u64().saturating_sub(1),
        "expires_at": crate::now_ts_u64().saturating_add(300),
    });
    let result_json = json!({"resume_context": {"approval_request": approval_request}});
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            updated_at TEXT
        );",
    )
    .expect("create tasks table");
    db.execute(
        "INSERT INTO tasks (task_id, status, result_json, updated_at)
         VALUES (?1, 'running', ?2, '1')",
        params![task.task_id, result_json.to_string()],
    )
    .expect("insert approved task");
    drop(db);

    let plan = super::tests::plan_result(vec![step]);
    let verify = || {
        verify_plan(
            &state,
            &task,
            VerifyInput {
                output_contract: Some(&super::tests::route_result()),
                request_text: None,
                context_bundle_summary: Some("resume"),
                plan_result: &plan,
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        )
    };

    let first = verify();
    assert!(first.approved);
    assert!(!first.needs_confirmation);
    assert!(first
        .issues
        .iter()
        .all(|issue| issue.kind != VerifyIssueKind::ConfirmationRequired));
    assert_eq!(
        first.permission_decision["approval_grant"]["status"],
        "consumed"
    );

    let second = verify();
    assert!(second.needs_confirmation);
    assert!(second
        .issues
        .iter()
        .any(|issue| issue.kind == VerifyIssueKind::ConfirmationRequired));
    assert_eq!(
        second.permission_decision["approval_grant"]["status"],
        crate::repo::TaskApprovalConsumeOutcome::NotApproved.as_token()
    );

    let db = state.core.db.get().expect("get db");
    let stored: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            params![task.task_id],
            |row| row.get(0),
        )
        .expect("select consumed approval");
    let stored: Value = serde_json::from_str(&stored).expect("stored result");
    assert_eq!(
        stored["resume_context"]["approval_request"]["status"],
        "consumed"
    );
}

#[test]
fn verifier_reuses_only_an_exact_active_scope_grant() {
    let mut state = super::tests::test_state();
    state.skill_rt.tools_policy = Arc::new(
        crate::ToolsPolicy::from_config(&ToolsConfig {
            sandbox_mode: ToolSandboxMode::WorkspaceWrite,
            approval_policy: ToolApprovalPolicy::Always,
            ..ToolsConfig::default()
        })
        .expect("approval policy"),
    );
    let mut task = super::tests::test_task();
    task.user_key = Some("actor-key".to_string());
    let step = scoped_confirmation_step("run/example.txt");
    let binding = crate::approval_grant::binding_for_confirmation_steps(
        &state,
        std::slice::from_ref(&step),
        &[step.step_id.clone()],
    )
    .expect("approval binding");
    let scope = binding.scope.as_ref().expect("grantable local scope");
    let verify_step = |step: PlanStep| {
        let plan = super::tests::plan_result(vec![step]);
        verify_plan(
            &state,
            &task,
            VerifyInput {
                output_contract: Some(&super::tests::route_result()),
                request_text: None,
                context_bundle_summary: Some("scope grant"),
                plan_result: &plan,
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        )
    };

    let without_grant = verify_step(step.clone());
    assert!(without_grant.needs_confirmation);

    let db = state.core.db.get().expect("get db");
    let grant = crate::repo::approval_scope::insert_approval_scope_grant(
        &db,
        "task-source",
        task.user_id,
        task.chat_id,
        &task.channel,
        task.user_key.as_deref().expect("actor key"),
        scope,
        crate::now_ts_u64() as i64,
    )
    .expect("insert scope grant");
    drop(db);

    let exact = verify_step(step.clone());
    assert!(exact.approved);
    assert!(!exact.needs_confirmation);
    assert_eq!(
        exact.permission_decision["approval_grant"]["status"],
        "scope_grant_matched"
    );

    let changed_resource = verify_step(scoped_confirmation_step("run/other.txt"));
    assert!(changed_resource.needs_confirmation);
    assert!(changed_resource
        .issues
        .iter()
        .any(|issue| issue.kind == VerifyIssueKind::ConfirmationRequired));

    assert!(
        crate::repo::revoke_approval_scope_grant(&state, "actor-key", &grant.grant_id)
            .expect("revoke scope grant")
    );
    let revoked = verify_step(step);
    assert!(revoked.needs_confirmation);
}
