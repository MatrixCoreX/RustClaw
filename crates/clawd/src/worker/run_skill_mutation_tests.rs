use super::{
    canonical_json_string, finalize_direct_run_skill_reconciliation,
    persist_direct_run_skill_mutation_result, prepare_direct_run_skill_mutation,
    DirectRunSkillMutationGuard,
};

fn task_fixture(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn insert_active_task(state: &crate::AppState, task: &crate::ClaimedTask) {
    let db = state.core.db.get().expect("test db");
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            updated_at INTEGER NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claim_attempt INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create tasks");
    db.execute(
        "INSERT OR REPLACE INTO tasks (
             task_id, status, result_json, updated_at, lease_owner, claim_attempt
         ) VALUES (?1, 'running', NULL, 0, ?2, ?3)",
        rusqlite::params![
            task.task_id,
            state.worker.worker_id.as_str(),
            task.claim_attempt
        ],
    )
    .expect("insert active task");
}

#[test]
fn direct_run_skill_mutation_persists_receipt_and_suppresses_retry() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task_fixture("task-direct-mutation");
    insert_active_task(&state, &task);
    let args = serde_json::json!({
        "action": "append_text",
        "path": "notes.txt",
        "content": "one"
    });
    let guard =
        prepare_direct_run_skill_mutation(&state, &task, "fs_basic", &args).expect("prepare");
    let DirectRunSkillMutationGuard::Acquired(lease) = &guard else {
        panic!("expected acquired mutation");
    };
    let execution = guard.execution_context().expect("execution context");
    assert_eq!(execution.idempotency_key, lease.record.idempotency_key);
    assert_eq!(execution.attempt_no, 1);

    let result = Ok(crate::skills::SkillRunOutcome {
        text: serde_json::json!({"status": "ok"}).to_string(),
        notify: None,
        validation: None,
        extra: Some(serde_json::json!({
            "schema_version": 1,
            "status_code": "mutation_applied",
            "text": "must not persist"
        })),
    });
    assert!(persist_direct_run_skill_mutation_result(
        &state, &guard, &result
    ));

    let retried =
        prepare_direct_run_skill_mutation(&state, &task, "fs_basic", &args).expect("retry");
    let DirectRunSkillMutationGuard::ReplaySuppressed(record) = retried else {
        panic!("completed direct mutation must suppress replay");
    };
    assert_eq!(
        record.phase,
        crate::repo::task_mutation_ledger::TaskMutationPhase::Committed
    );
    assert_eq!(
        record
            .receipt
            .as_ref()
            .and_then(|value| value.pointer("/structured_extra/status_code"))
            .and_then(serde_json::Value::as_str),
        Some("mutation_applied")
    );
    assert!(!record
        .receipt
        .expect("receipt")
        .to_string()
        .contains("must not persist"));
}

#[test]
fn direct_run_skill_ambiguous_failure_checkpoints_instead_of_terminal_retry() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task_fixture("task-direct-uncertain");
    insert_active_task(&state, &task);
    let args = serde_json::json!({
        "action": "append_text",
        "path": "notes.txt",
        "content": "one"
    });
    let guard =
        prepare_direct_run_skill_mutation(&state, &task, "fs_basic", &args).expect("prepare");
    let DirectRunSkillMutationGuard::Acquired(lease) = &guard else {
        panic!("expected acquired mutation");
    };
    let result = Err("adapter_transport_lost".to_string());
    assert!(!persist_direct_run_skill_mutation_result(
        &state, &guard, &result
    ));
    finalize_direct_run_skill_reconciliation(
        &state,
        &task,
        "fs_basic",
        &lease.record.action_ref,
        &lease.record.fingerprint_hash,
    )
    .expect("checkpoint reconciliation");

    let db = state.core.db.get().expect("test db");
    let (status, result_json): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task.task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load checkpointed task");
    assert_eq!(status, "running");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse checkpoint");
    assert_eq!(
        result
            .pointer("/task_lifecycle/state")
            .and_then(serde_json::Value::as_str),
        Some("needs_user")
    );
    assert_eq!(
        result
            .pointer("/task_checkpoint/pending_action/kind")
            .and_then(serde_json::Value::as_str),
        Some("mutation_reconciliation")
    );
    assert!(result.get("text").is_none());
    assert!(result.get("error_text").is_none());
}

#[test]
fn direct_run_skill_fingerprint_is_independent_of_object_key_order() {
    let left = serde_json::json!({
        "action": "apply",
        "path": "config.toml",
        "value": {"enabled": true, "count": 2}
    });
    let right: serde_json::Value = serde_json::from_str(
        r#"{"value":{"count":2,"enabled":true},"path":"config.toml","action":"apply"}"#,
    )
    .expect("parse reordered args");

    assert_eq!(canonical_json_string(&left), canonical_json_string(&right));
}
