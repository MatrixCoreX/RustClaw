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
