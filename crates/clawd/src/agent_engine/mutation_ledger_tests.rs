use super::{
    prepare_mutation_execution, record_completed_without_replay, safe_mutation_outcome_projection,
    MutationExecutionGuard,
};

fn task_fixture() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "task-mutation-policy".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn observation_does_not_create_mutation_ledger_entry() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let outcome = prepare_mutation_execution(
        &state,
        &task_fixture(),
        "fs_basic",
        &serde_json::json!({"action": "read_text_range", "path": "README.md"}),
        "skill:fs_basic:read",
        crate::execution_recipe::ActionEffect::observe(),
    )
    .expect("prepare observation");
    assert!(matches!(outcome, MutationExecutionGuard::NotRequired));
}

#[test]
fn unclassified_mutation_fails_closed_into_ledger() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let outcome = prepare_mutation_execution(
        &state,
        &task_fixture(),
        "fs_basic",
        &serde_json::json!({"action": "append_text", "path": "notes.txt"}),
        "skill:fs_basic:append",
        crate::execution_recipe::ActionEffect::mutate(),
    )
    .expect("prepare mutation");
    assert!(matches!(outcome, MutationExecutionGuard::Acquired(_)));
}

#[test]
fn mutation_projection_keeps_async_resume_contract_and_drops_user_content() {
    let projection = safe_mutation_outcome_projection(Some(&serde_json::json!({
        "source": "fixture",
        "status": "accepted",
        "text": "must not persist",
        "secret": "must not persist",
        "pending_async_job": {
            "job_id": "provider:video_generate:minimax:job-1",
            "provider": "minimax",
            "status": "accepted",
            "poll_after_seconds": 5,
            "expires_at": 2_000_000_000,
            "cancel_ref": "provider:video_generate:minimax:job-1",
            "message_key": "clawd.task.async_job_pending",
            "poll_adapter": {
                "kind": "media_job_poll",
                "skill_name": "video_generate",
                "args": {
                    "action": "poll",
                    "task_id": "job-1",
                    "job_id": "provider:video_generate:minimax:job-1",
                    "output_path": "video/job-1.mp4",
                    "prompt": "must not persist",
                    "api_key": "must not persist",
                    "command": "must not persist"
                }
            }
        }
    })))
    .expect("safe projection");

    assert_eq!(
        projection["structured_extra"]["pending_async_job"]["job_id"],
        "provider:video_generate:minimax:job-1"
    );
    assert_eq!(
        projection["structured_extra"]["poll_adapter"]["args"]["output_path"],
        "video/job-1.mp4"
    );
    let serialized = projection.to_string();
    assert!(!serialized.contains("must not persist"));
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("prompt"));
    assert!(!serialized.contains("command"));
}

#[test]
fn completed_async_mutation_rebuilds_waiting_checkpoint_without_replay() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task_fixture();
    {
        let db = state.core.db.get().expect("test db");
        db.execute_batch(
            "CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                result_json TEXT,
                updated_at INTEGER NOT NULL
             );",
        )
        .expect("create task progress table");
        db.execute(
            "INSERT INTO tasks (task_id, status, result_json, updated_at)
             VALUES (?1, 'running', NULL, 0)",
            rusqlite::params![task.task_id],
        )
        .expect("insert running task");
    }
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.round_no = 1;
    let record = crate::repo::TaskMutationRecord {
        task_id: task.task_id.clone(),
        fingerprint_hash: "fingerprint-hash".to_string(),
        action_ref: "skill:run_cmd:action:async_start".to_string(),
        status: "completed".to_string(),
        outcome: safe_mutation_outcome_projection(Some(&serde_json::json!({
            "pending_async_job": {
                "job_id": "local_process:/tmp/rustclaw-job-1",
                "status": "accepted",
                "poll_after_seconds": 1,
                "expires_at": 2_000_000_000,
                "cancel_ref": "local_process:/tmp/rustclaw-job-1",
                "message_key": "clawd.task.async_job_pending"
            }
        }))),
    };

    let outcome = record_completed_without_replay(
        &state,
        &task,
        &mut loop_state,
        &record,
        "skill:run_cmd:async_start",
        "run_cmd",
        1,
        1,
    )
    .expect("rebuild async checkpoint");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("async_job_checkpoint_waiting")
    );
    assert!(!outcome.continue_in_round);
    assert_eq!(
        loop_state
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.pointer("/pending_async_job/job_id"))
            .and_then(serde_json::Value::as_str),
        Some("local_process:/tmp/rustclaw-job-1")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.mutation_replay_suppressed")
            .map(String::as_str),
        Some("true")
    );
}
