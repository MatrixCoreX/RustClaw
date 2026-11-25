//! Exercise the real terminal projection and memory outbox in one SQLite transaction.
use super::*;
use crate::memory::capacity_test_fixtures::Fixture;
use rusqlite::params;
use serde_json::Value;

const TASK: &str = "resumed-memory-turn";
const CHECKPOINT: &str = "resumed-memory-checkpoint";
const NOW: i64 = 12_000;

fn fixture() -> Fixture {
    let mut f = Fixture::file();
    f.state_mut().policy.memory.long_term_enabled = true;
    f.state_mut().policy.memory.enable_preference_extraction = true;
    f.state_mut().policy.memory.llm_preference_fallback_enabled = true;
    f
}

fn claim(f: &Fixture, outcome: &str) -> Value {
    let seed = terminal_projection_seed(
        TASK,
        CHECKPOINT,
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        "ready_to_run_seeded_agent_loop",
        outcome,
        NOW,
    );
    {
        let db = f.state().core.db.get().unwrap();
        db.execute(
            "INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,
            kind,payload_json,status,result_json,created_at,updated_at)
            VALUES (?1,1,7,?2,?3,'ui','ask',?4,'running',?5,?6,?6)",
            params![
                TASK,
                f.key,
                f.principal,
                json!({"text":"Read the requested range exactly once."}).to_string(),
                seed.to_string(),
                NOW.to_string()
            ],
        )
        .unwrap();
    }
    activate_resume_owner(f.state(), TASK, CHECKPOINT, NOW, NOW + 30);
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        f.state(),
        TASK,
        CHECKPOINT,
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        "ready_to_run_seeded_agent_loop",
        outcome,
        NOW + 1,
        20,
    )
    .unwrap()
    .expect("real projection claim");
    json!({"schema_version":1,"task_id":TASK,"checkpoint_id":CHECKPOINT,
        "executor_state":"executing_planner_resume","executor_action":"run_seeded_agent_loop",
        "executor_status":"seeded_loop_requires_provider_window",
        "dispatch_state":"ready_to_run_seeded_agent_loop","executor_result_status":outcome,
        "result_projection_state":if outcome=="seeded_loop_completed" {
            "project_seeded_loop_completed"
        } else {"project_seeded_loop_failed"},
        "final_result_json":{"text":"The component is declared on line 9."},
        "error_code":"fixture_provider_unavailable","message_key":"fixture.provider_unavailable"})
}

fn project(f: &Fixture, payload: &Value) -> anyhow::Result<bool> {
    record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
        f.state(),
        CLAIM_ATTEMPT,
        TASK,
        CHECKPOINT,
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        "ready_to_run_seeded_agent_loop",
        payload["executor_result_status"].as_str().unwrap(),
        payload,
        NOW + 2,
    )
}

fn counts(f: &Fixture) -> (i64, i64, i64) {
    let db = f.state().core.db.get().unwrap();
    (
        db.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap(),
        db.query_row(
            "SELECT COUNT(*) FROM memory_source_events WHERE source_task_id=?1",
            [TASK],
            |r| r.get(0),
        )
        .unwrap(),
        db.query_row(
            "SELECT COUNT(*) FROM memory_jobs WHERE source_task_id=?1",
            [TASK],
            |r| r.get(0),
        )
        .unwrap(),
    )
}

#[test]
fn recovered_success_commits_owned_memory_sources_and_jobs_exactly_once() {
    let f = fixture();
    let payload = claim(&f, "seeded_loop_completed");
    assert!(project(&f, &payload).unwrap());
    assert_eq!(
        stored_task_status_error_result(f.state(), TASK).0,
        "succeeded"
    );
    assert_eq!(counts(&f), (2, 2, 2));
    assert!(
        !project(&f, &payload).unwrap(),
        "consumed result cannot enqueue twice"
    );
    assert_eq!(counts(&f), (2, 2, 2));
    let db = f.state().core.db.get().unwrap();
    let rows: Vec<(String, String, String)> = db
        .prepare(
            "SELECT role,content,principal_id
        FROM memories ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows[0],
        (
            "user".into(),
            "Read the requested range exactly once.".into(),
            f.principal.clone()
        )
    );
    assert!(rows[1].1.ends_with("The component is declared on line 9."));
    assert_eq!(rows[1].2, f.principal);
}

#[test]
fn memory_write_failure_rolls_back_completion_and_can_retry_without_duplicate_sources() {
    let f = fixture();
    let payload = claim(&f, "seeded_loop_completed");
    f.state().core.db.get().unwrap().execute_batch("CREATE TRIGGER fail_memory_source
        BEFORE INSERT ON memory_source_events BEGIN SELECT RAISE(ABORT,'injected_memory_write_failure'); END;").unwrap();
    assert!(project(&f, &payload).is_err());
    assert_eq!(
        stored_task_status_error_result(f.state(), TASK).0,
        "running"
    );
    assert_eq!(
        counts(&f),
        (0, 0, 0),
        "partial source pair must roll back with task status"
    );
    f.state()
        .core
        .db
        .get()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_memory_source")
        .unwrap();
    assert!(project(&f, &payload).unwrap());
    assert_eq!(counts(&f), (2, 2, 2));
}

#[test]
fn lost_terminal_claim_does_not_write_memory() {
    let f = fixture();
    let payload = claim(&f, "seeded_loop_completed");
    f.state()
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='canceled' WHERE task_id=?1",
            [TASK],
        )
        .unwrap();
    assert!(!project(&f, &payload).unwrap());
    assert_eq!(counts(&f), (0, 0, 0));
}

#[test]
fn failed_recovery_does_not_generate_success_memory() {
    let f = fixture();
    let payload = claim(&f, "seeded_loop_failed");
    assert!(project(&f, &payload).unwrap());
    assert_eq!(stored_task_status_error_result(f.state(), TASK).0, "failed");
    assert_eq!(counts(&f), (0, 0, 0));
}

#[test]
fn recovered_success_respects_explicit_generation_opt_out() {
    let f = fixture();
    let payload = claim(&f, "seeded_loop_completed");
    let actor = crate::resolve_auth_identity_by_key(f.state(), &f.key)
        .unwrap()
        .unwrap();
    {
        use crate::memory::settings::*;
        update_memory_settings(
            &f.state().core.db.get().unwrap(),
            &actor,
            &MemorySettingsUpdateRequest {
                scope: MemorySettingScope::Principal,
                target_principal_id: None,
                conversation_id: None,
                use_mode: None,
                generate_mode: Some(MemorySettingMode::Disabled),
                external_context_policy: None,
                expected_revision: Some(0),
                long_term_enabled: None,
            },
            true,
        )
        .unwrap();
    }
    assert!(project(&f, &payload).unwrap());
    assert_eq!(counts(&f).2, 0);
}
