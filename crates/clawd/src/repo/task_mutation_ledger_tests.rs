use super::{
    begin_task_mutation, complete_task_mutation, mark_task_mutation_uncertain,
    BeginTaskMutationOutcome, TaskMutationClaimRejected, TaskMutationLease,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

const TEST_WORKER_ID: &str = "worker:test";
const TEST_CLAIM_ATTEMPT: i64 = 1;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw-task-mutation-ledger-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create temp directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn file_pool(path: &Path) -> crate::db_init::DbPool {
    Pool::builder()
        .max_size(8)
        .build(SqliteConnectionManager::file(path))
        .expect("build sqlite pool")
}

fn acquired(outcome: BeginTaskMutationOutcome) -> TaskMutationLease {
    match outcome {
        BeginTaskMutationOutcome::Acquired(lease) => lease,
        other => panic!("expected acquired mutation lease, got {other:?}"),
    }
}

fn insert_active_task(pool: &crate::db_init::DbPool, task_id: &str) {
    let db = pool.get().expect("get test db");
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            lease_owner TEXT,
            claim_attempt INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create task claim table");
    db.execute(
        "INSERT OR REPLACE INTO tasks (task_id, status, lease_owner, claim_attempt)
         VALUES (?1, 'running', ?2, ?3)",
        rusqlite::params![task_id, TEST_WORKER_ID, TEST_CLAIM_ATTEMPT],
    )
    .expect("insert active task claim");
}

#[test]
fn completed_mutation_is_not_acquired_again() {
    let temp = TempDir::new();
    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    insert_active_task(&pool, "task-completed");
    let lease = acquired(
        begin_task_mutation(
            &pool,
            TEST_WORKER_ID,
            TEST_CLAIM_ATTEMPT,
            "task-completed",
            "skill:config_edit:action:apply",
            "skill:config_edit:action:apply",
        )
        .expect("begin mutation"),
    );
    complete_task_mutation(
        &pool,
        &lease,
        r#"{"status":"ok"}"#,
        Some(&serde_json::json!({
            "schema_version": 1,
            "structured_extra": {
                "status": "ok",
                "status_code": "mutation_completed"
            }
        })),
    )
    .expect("complete mutation");

    let duplicate = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        "task-completed",
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("read completed mutation");
    let BeginTaskMutationOutcome::Completed(record) = duplicate else {
        panic!("expected completed mutation");
    };
    assert_eq!(
        record
            .outcome
            .as_ref()
            .and_then(|value| value.pointer("/structured_extra/status_code"))
            .and_then(serde_json::Value::as_str),
        Some("mutation_completed")
    );
}

#[test]
fn response_loss_restart_leaves_mutation_uncertain_instead_of_reacquiring() {
    let temp = TempDir::new();
    let db_path = temp.0.join("tasks.sqlite");
    let marker_path = temp.0.join("mutation-marker");
    let first_pool = file_pool(&db_path);
    insert_active_task(&first_pool, "task-response-loss");
    let _lease = acquired(
        begin_task_mutation(
            &first_pool,
            TEST_WORKER_ID,
            TEST_CLAIM_ATTEMPT,
            "task-response-loss",
            "skill:fs_basic:write_text",
            "skill:fs_basic:action:write_text",
        )
        .expect("begin mutation"),
    );
    std::fs::write(&marker_path, "mutation-once\n").expect("simulate completed external mutation");
    drop(first_pool);

    let restarted_pool = file_pool(&db_path);
    let resumed = begin_task_mutation(
        &restarted_pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        "task-response-loss",
        "skill:fs_basic:write_text",
        "skill:fs_basic:action:write_text",
    )
    .expect("inspect mutation after restart");
    assert!(matches!(resumed, BeginTaskMutationOutcome::Uncertain(_)));
    assert_eq!(
        std::fs::read_to_string(marker_path).expect("read mutation marker"),
        "mutation-once\n"
    );
}

#[test]
fn concurrent_duplicate_begins_have_one_owner() {
    let temp = TempDir::new();
    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    insert_active_task(&pool, "task-concurrent");
    let barrier = Arc::new(Barrier::new(8));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            begin_task_mutation(
                &pool,
                TEST_WORKER_ID,
                TEST_CLAIM_ATTEMPT,
                "task-concurrent",
                "skill:package_manager:install",
                "skill:package_manager:action:install",
            )
            .expect("concurrent begin")
        }));
    }
    let outcomes = threads
        .into_iter()
        .map(|thread| thread.join().expect("join mutation begin"))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginTaskMutationOutcome::Acquired(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginTaskMutationOutcome::Uncertain(_)))
            .count(),
        7
    );
}

#[test]
fn stale_generation_cannot_commit_or_reclassify_mutation_receipt() {
    let temp = TempDir::new();
    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    let task_id = "task-generation-fence";
    insert_active_task(&pool, task_id);
    let lease = acquired(
        begin_task_mutation(
            &pool,
            TEST_WORKER_ID,
            TEST_CLAIM_ATTEMPT,
            task_id,
            "skill:config_edit:action:apply",
            "skill:config_edit:action:apply",
        )
        .expect("begin mutation"),
    );
    {
        let db = pool.get().expect("get test db");
        db.execute(
            "UPDATE tasks SET claim_attempt = 2 WHERE task_id = ?1",
            rusqlite::params![task_id],
        )
        .expect("advance task generation");
    }

    let completion_error = complete_task_mutation(
        &pool,
        &lease,
        r#"{"status":"ok"}"#,
        Some(&serde_json::json!({"status_code": "mutation_completed"})),
    )
    .expect_err("stale generation completion must be fenced");
    let rejection = completion_error
        .downcast_ref::<TaskMutationClaimRejected>()
        .expect("typed claim rejection");
    assert_eq!(
        rejection.status_code,
        crate::repo::WORKER_LEASE_LOST_STATUS_CODE
    );
    assert_eq!(rejection.expected_claim_attempt, 1);
    assert_eq!(rejection.active_claim_attempt, Some(2));

    let uncertain_error = mark_task_mutation_uncertain(&pool, &lease)
        .expect_err("stale generation uncertainty write must be fenced");
    assert!(uncertain_error
        .downcast_ref::<TaskMutationClaimRejected>()
        .is_some());
    let stale_begin = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect_err("stale generation begin must be fenced");
    assert!(stale_begin
        .downcast_ref::<TaskMutationClaimRejected>()
        .is_some());

    let resumed = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        2,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("new generation reads prior receipt");
    assert!(matches!(resumed, BeginTaskMutationOutcome::Uncertain(_)));
}
