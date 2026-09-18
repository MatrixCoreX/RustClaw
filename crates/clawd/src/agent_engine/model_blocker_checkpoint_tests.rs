use super::*;
use crate::agent_engine::seed_loop_state_from_task_checkpoint;
use crate::executor::{StepExecutionResult, StepExecutionStatus};

fn fixture() -> (AppState, ClaimedTask, LoopState) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "blocked-loop-evidence".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    state.core.db.get().unwrap().execute(
        "INSERT INTO tasks (task_id,user_id,chat_id,kind,payload_json,status,created_at,updated_at,lease_owner,claim_attempt) VALUES (?1,1,2,'ask','{}','running',0,0,?2,1)",
        rusqlite::params![task.task_id, state.worker.worker_id.as_str()],
    ).unwrap();
    let mut loop_state = LoopState::new();
    loop_state.round_no = 3;
    loop_state.total_steps_executed = 2;
    loop_state.tool_calls_total = 2;
    loop_state
        .loaded_capability_skills
        .insert("fs_basic".to_string());
    loop_state.last_written_file_path = Some("tmp/partial.txt".to_string());
    loop_state
        .history_compact
        .push("executed-step-record".to_string());
    loop_state
        .successful_action_fingerprints
        .insert("write-fingerprint".to_string(), 1);
    loop_state
        .task_observations
        .push(json!({"mutation_id":"mutation-1","status":"ok"}));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "write_file".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(json!({"path":"tmp/partial.txt","content_bytes":18}).to_string()),
        error: None,
        started_at: 10,
        finished_at: 12,
    });
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut loop_state,
        "write_file",
        "write-fingerprint",
        StepExecutionStatus::Ok,
        "observed",
        None,
        "",
    );
    (state, task, loop_state)
}

fn block(state: &AppState, task: &ClaimedTask) {
    state.note_task_provider_blocker(
        &task.task_id,
        crate::TaskProviderBlocker {
            provider: "fixture".to_string(),
            status_code: "quota_exhausted".to_string(),
            retry_after_seconds: 4500,
            external_provider_blocked: true,
            message_key: "provider.quota_exhausted".to_string(),
        },
    );
}

#[test]
fn provider_wait_retains_execution_evidence_and_resume_fingerprints() {
    let (state, task, mut loop_state) = fixture();
    block(&state, &task);
    for _ in 0..3 {
        state.note_task_llm_call_with_label_and_prompt_size(&task.task_id, "plan", 10);
    }
    let before = crate::now_ts_u64() as i64;
    assert!(checkpoint_blocked_model_error(&state, &task, &mut loop_state).unwrap());
    let lifecycle = loop_state.task_lifecycle.as_ref().unwrap();
    assert_eq!(lifecycle["state"], "waiting");
    assert_eq!(
        lifecycle["provider_status"]["status_code"],
        "quota_exhausted"
    );
    assert!(lifecycle["next_check_after"].as_i64().unwrap() >= before + 4500);
    let checkpoint: crate::task_lifecycle::TaskCheckpoint =
        serde_json::from_value(loop_state.task_checkpoint.clone().unwrap()).unwrap();
    assert_eq!(checkpoint.budget.round, 3);
    assert_eq!(checkpoint.budget.step, 2);
    assert_eq!(checkpoint.budget.tool_calls, 2);
    assert_eq!(checkpoint.budget.llm_calls, 3);
    assert_eq!(checkpoint.budget.tool_elapsed_ms, 2000);
    assert_eq!(checkpoint.evidence_refs, vec!["step_2"]);
    assert_eq!(
        checkpoint.completed_side_effect_refs,
        vec!["write-fingerprint"]
    );
    assert!(checkpoint
        .artifact_refs
        .contains(&"changed_file:tmp/partial.txt".to_string()));
    assert!(checkpoint.attempt_ledger.is_some());
    let mut resumed = LoopState::new();
    seed_loop_state_from_task_checkpoint(&mut resumed, &checkpoint);
    assert_eq!(resumed.tool_calls_total, 2);
    assert_eq!(resumed.attempt_ledger_entries.len(), 1);
    assert_eq!(resumed.attempt_ledger_entries[0].attempt_id, "a1");
    assert_eq!(resumed.attempt_ledger_entries[0].status, "ok");
    assert_eq!(resumed.executed_step_results.len(), 1);
    assert_eq!(
        resumed.executed_step_results[0].output,
        loop_state.executed_step_results[0].output
    );
    assert_eq!(
        resumed.successful_action_fingerprints["write-fingerprint"],
        1
    );
    assert!(resumed.loaded_capability_skills.contains("fs_basic"));
    assert!(resumed
        .task_observations
        .contains(&json!({"mutation_id":"mutation-1","status":"ok"})));
    let persisted: String = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id=?1",
            [&task.task_id],
            |row| row.get(0),
        )
        .unwrap();
    let persisted: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(persisted["task_checkpoint"], checkpoint.to_machine_json());
}

#[test]
fn no_machine_blocker_leaves_failure_and_state_untouched() {
    let (state, task, mut loop_state) = fixture();
    assert!(!checkpoint_blocked_model_error(&state, &task, &mut loop_state).unwrap());
    assert!(loop_state.task_checkpoint.is_none());
}

#[test]
fn canceled_claim_does_not_become_provider_wait() {
    let (state, task, mut loop_state) = fixture();
    block(&state, &task);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='canceled' WHERE task_id=?1",
            [&task.task_id],
        )
        .unwrap();
    assert_eq!(
        checkpoint_blocked_model_error(&state, &task, &mut loop_state).unwrap_err(),
        crate::agent_engine::TASK_CANCELED_ERR
    );
    assert!(loop_state.task_checkpoint.is_none());
}

#[test]
fn cost_wait_preserves_the_same_executed_state() {
    let (state, task, mut loop_state) = fixture();
    state.metrics.cost_blocker_per_task.lock().unwrap().insert(
        task.task_id.clone(),
        crate::TaskCostBlocker {
            status_code: "llm_cost_hard_ceiling".to_string(),
            scope: "task".to_string(),
            observed_cost_usd_nanos: 6,
            limit_cost_usd_nanos: 5,
            retry_after_seconds: 60,
            message_key: "llm.cost_hard_ceiling".to_string(),
        },
    );
    assert!(checkpoint_blocked_model_error(&state, &task, &mut loop_state).unwrap());
    let checkpoint = loop_state.task_checkpoint.unwrap();
    assert_eq!(checkpoint["budget"]["tool_calls"], 2);
    assert_eq!(
        checkpoint["repair_signal"]["policy_status"]["status_code"],
        "llm_cost_hard_ceiling"
    );
    assert_eq!(
        checkpoint["completed_side_effect_refs"],
        json!(["write-fingerprint"])
    );
}

#[test]
fn repeated_checkpoint_restores_typed_failure_policy_and_deduplicates_attempts() {
    let (state, task, mut loop_state) = fixture();
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut loop_state,
        "fixture_tool",
        "action=lookup",
        StepExecutionStatus::Error,
        "",
        None,
        &crate::skills::structured_skill_error_from_parts(
            "fixture_tool",
            "provider_retryable_response",
            "fixture",
            None,
            Some(
                json!({"provider":"fixture","provider_error_class":"rate_limited",
                        "external_provider_blocked":true,"retry_after_seconds":60}),
            ),
        ),
    );
    block(&state, &task);
    assert!(checkpoint_blocked_model_error(&state, &task, &mut loop_state).unwrap());
    let checkpoint: crate::task_lifecycle::TaskCheckpoint =
        serde_json::from_value(loop_state.task_checkpoint.clone().unwrap()).unwrap();
    let mut resumed = LoopState::new();
    seed_loop_state_from_task_checkpoint(&mut resumed, &checkpoint);
    let expected = crate::agent_engine::attempt_ledger::build_attempt_ledger_snapshot(&loop_state);
    assert_eq!(
        crate::agent_engine::attempt_ledger::build_attempt_ledger_snapshot(&resumed),
        expected
    );
    crate::agent_engine::attempt_ledger::restore_attempt_ledger_snapshot(
        &mut resumed,
        checkpoint.attempt_ledger.as_ref().unwrap(),
    );
    assert_eq!(resumed.attempt_ledger_entries.len(), 2);
    assert!(checkpoint_blocked_model_error(&state, &task, &mut resumed).unwrap());
    assert_eq!(
        resumed.task_checkpoint.unwrap()["attempt_ledger"],
        expected.unwrap()
    );
}

#[test]
fn committed_file_effect_is_not_replayed_after_provider_wait_and_concurrent_edit() {
    use crate::agent_engine::mutation_ledger::{
        complete_mutation_execution, prepare_mutation_execution, MutationExecutionGuard,
    };
    let (state, task, mut loop_state) = fixture();
    let root = std::env::temp_dir().join(format!("checkpoint-effect-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let path = root.join("partial.txt");
    let args = json!({"path":path,"content":"original"});
    let fingerprint = "write-fingerprint";
    let MutationExecutionGuard::Acquired(lease) = prepare_mutation_execution(
        &state,
        &task,
        "write_file",
        &args,
        fingerprint,
        crate::execution_recipe::ActionEffect::mutate(),
    )
    .unwrap() else {
        panic!("initial mutation must acquire lease")
    };
    std::fs::write(&path, b"original").unwrap();
    assert!(complete_mutation_execution(
        &state,
        &lease,
        r#"{"status":"ok"}"#,
        Some(&json!({"status":"ok"})),
        &crate::execution_recipe::ValidationObservation::Passed,
        false,
    ));
    block(&state, &task);
    assert!(checkpoint_blocked_model_error(&state, &task, &mut loop_state).unwrap());
    let checkpoint = serde_json::from_value(loop_state.task_checkpoint.unwrap()).unwrap();
    let mut resumed = LoopState::new();
    seed_loop_state_from_task_checkpoint(&mut resumed, &checkpoint);
    std::fs::write(&path, b"concurrent-change").unwrap();
    assert!(matches!(
        prepare_mutation_execution(
            &state,
            &task,
            "write_file",
            &args,
            fingerprint,
            crate::execution_recipe::ActionEffect::mutate(),
        )
        .unwrap(),
        MutationExecutionGuard::Completed(_)
    ));
    assert_eq!(resumed.successful_action_fingerprints[fingerprint], 1);
    assert_eq!(std::fs::read(&path).unwrap(), b"concurrent-change");
    let attempts: i64 = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT attempt_no FROM task_mutation_ledger WHERE task_id=?1",
            [&task.task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 1);
}
