use super::{
    begin_task_mutation, commit_task_mutation, mark_task_mutation_uncertain,
    reconcile_task_mutation, record_task_mutation_receipt, record_task_mutation_verification,
    start_task_mutation_attempt, BeginTaskMutationOutcome, ReconcileTaskMutationOutcome,
    TaskMutationClaimRejected, TaskMutationLease, TaskMutationPhase, TaskMutationReconciliation,
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
fn initial_migration_remains_restart_safe_after_ledger_schema_upgrade() {
    let mut db = rusqlite::Connection::open_in_memory().expect("open database");
    db.execute_batch(crate::INIT_SQL)
        .expect("apply initial migration");
    super::ensure_task_mutation_ledger_schema(&mut db).expect("ensure v2 ledger schema");

    db.execute_batch(crate::INIT_SQL)
        .expect("reapply initial migration after ledger upgrade");
    assert!(super::table_has_column(&db, "task_mutation_ledger", "phase")
        .expect("inspect ledger columns"));
    assert!(!super::table_has_column(&db, "task_mutation_ledger", "status")
        .expect("inspect legacy ledger column"));
}

#[test]
fn completed_mutation_is_not_acquired_again() {
    let temp = TempDir::new();
    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    insert_active_task(&pool, "task-completed");
    let mut lease = acquired(
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
    start_task_mutation_attempt(&pool, &mut lease).expect("start mutation attempt");
    record_task_mutation_receipt(
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
    .expect("record mutation receipt");
    record_task_mutation_verification(
        &pool,
        &lease,
        &serde_json::json!({"schema_version": 1, "status": "passed"}),
        true,
    )
    .expect("record mutation verification");
    commit_task_mutation(&pool, &lease).expect("commit mutation");

    let duplicate = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        "task-completed",
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("read completed mutation");
    let BeginTaskMutationOutcome::ReplaySuppressed(record) = duplicate else {
        panic!("expected completed mutation");
    };
    assert_eq!(
        record
            .receipt
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
    let mut lease = acquired(
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
    start_task_mutation_attempt(&first_pool, &mut lease).expect("start mutation attempt");
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
    assert!(matches!(
        resumed,
        BeginTaskMutationOutcome::ReconciliationRequired(_)
    ));
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
            .filter(|outcome| {
                matches!(outcome, BeginTaskMutationOutcome::ReconciliationRequired(_))
            })
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
    let mut lease = acquired(
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
    start_task_mutation_attempt(&pool, &mut lease).expect("start mutation attempt");
    {
        let db = pool.get().expect("get test db");
        db.execute(
            "UPDATE tasks SET claim_attempt = 2 WHERE task_id = ?1",
            rusqlite::params![task_id],
        )
        .expect("advance task generation");
    }

    let completion_error = record_task_mutation_receipt(
        &pool,
        &lease,
        r#"{"status":"ok"}"#,
        Some(&serde_json::json!({"status_code": "mutation_completed"})),
    )
    .expect_err("stale generation receipt must be fenced");
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
    assert!(matches!(
        resumed,
        BeginTaskMutationOutcome::ReconciliationRequired(_)
    ));
}

#[test]
fn deterministic_key_and_every_durable_phase_survive_database_reopen() {
    let temp = TempDir::new();
    let db_path = temp.0.join("tasks.sqlite");
    let pool = file_pool(&db_path);
    let task_id = "task-phase-restart";
    insert_active_task(&pool, task_id);
    let mut lease = acquired(
        begin_task_mutation(
            &pool,
            TEST_WORKER_ID,
            TEST_CLAIM_ATTEMPT,
            task_id,
            "skill:config_edit:action:apply",
            "skill:config_edit:action:apply",
        )
        .expect("record intent"),
    );
    let stable_key = lease.record.idempotency_key.clone();
    assert_eq!(lease.record.phase, TaskMutationPhase::IntentRecorded);

    start_task_mutation_attempt(&pool, &mut lease).expect("start attempt");
    assert_eq!(lease.record.phase, TaskMutationPhase::AttemptStarted);
    drop(pool);

    let pool = file_pool(&db_path);
    let after_attempt = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("inspect attempt after reopen");
    assert!(matches!(
        after_attempt,
        BeginTaskMutationOutcome::ReconciliationRequired(_)
    ));
    mark_task_mutation_uncertain(&pool, &lease).expect("mark reconciliation required");
    drop(pool);

    let pool = file_pool(&db_path);
    let reconciled = reconcile_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        &lease.record.fingerprint_hash,
        TaskMutationReconciliation::NotApplied,
        &serde_json::json!({
            "schema_version": 1,
            "disposition": "not_applied",
            "status_code": "provider_operation_absent"
        }),
    )
    .expect("reconcile not applied");
    let ReconcileTaskMutationOutcome::RetryReady(mut retry_lease) = reconciled else {
        panic!("expected retry-ready reconciliation");
    };
    assert_eq!(retry_lease.record.idempotency_key, stable_key);
    start_task_mutation_attempt(&pool, &mut retry_lease).expect("start retry attempt");
    record_task_mutation_receipt(
        &pool,
        &retry_lease,
        r#"{"status":"ok"}"#,
        Some(&serde_json::json!({
            "schema_version": 1,
            "structured_extra": {"status_code": "mutation_applied"}
        })),
    )
    .expect("record receipt");
    drop(pool);

    let pool = file_pool(&db_path);
    let after_receipt = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("inspect receipt after reopen");
    let BeginTaskMutationOutcome::ReplaySuppressed(receipt_record) = after_receipt else {
        panic!("receipt must suppress replay");
    };
    assert_eq!(receipt_record.phase, TaskMutationPhase::ReceiptRecorded);
    assert_eq!(receipt_record.idempotency_key, stable_key);

    record_task_mutation_verification(
        &pool,
        &retry_lease,
        &serde_json::json!({"schema_version": 1, "status": "passed"}),
        true,
    )
    .expect("record verification");
    drop(pool);

    let pool = file_pool(&db_path);
    let after_verification = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("inspect verification after reopen");
    let BeginTaskMutationOutcome::ReplaySuppressed(verified_record) = after_verification else {
        panic!("verified mutation must suppress replay");
    };
    assert_eq!(verified_record.phase, TaskMutationPhase::Verified);

    commit_task_mutation(&pool, &retry_lease).expect("commit mutation");
    drop(pool);

    let pool = file_pool(&db_path);
    let committed = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("inspect commit after reopen");
    let BeginTaskMutationOutcome::ReplaySuppressed(committed_record) = committed else {
        panic!("committed mutation must suppress replay");
    };
    assert_eq!(committed_record.phase, TaskMutationPhase::Committed);
    assert_eq!(committed_record.idempotency_key, stable_key);
    assert_eq!(committed_record.attempt_no, 2);
}

#[test]
fn intent_only_restart_can_transfer_to_new_claim_without_replaying_an_attempt() {
    let temp = TempDir::new();
    let db_path = temp.0.join("tasks.sqlite");
    let pool = file_pool(&db_path);
    let task_id = "task-intent-restart";
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
        .expect("record intent"),
    );
    let stable_key = lease.record.idempotency_key;
    drop(pool);

    let pool = file_pool(&db_path);
    {
        let db = pool.get().expect("restart db");
        db.execute(
            "UPDATE tasks
             SET lease_owner = 'worker:restart', claim_attempt = 2
             WHERE task_id = ?1",
            rusqlite::params![task_id],
        )
        .expect("transfer task claim");
    }
    let resumed = begin_task_mutation(
        &pool,
        "worker:restart",
        2,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("resume intent under new claim");
    let BeginTaskMutationOutcome::Acquired(resumed_lease) = resumed else {
        panic!("intent without an attempt must be safely reacquired");
    };
    assert_eq!(
        resumed_lease.record.phase,
        TaskMutationPhase::IntentRecorded
    );
    assert_eq!(resumed_lease.record.attempt_no, 0);
    assert_eq!(resumed_lease.record.idempotency_key, stable_key);
}

#[test]
fn verification_pending_restart_suppresses_original_mutation_replay() {
    let temp = TempDir::new();
    let db_path = temp.0.join("tasks.sqlite");
    let pool = file_pool(&db_path);
    let task_id = "task-verification-pending";
    insert_active_task(&pool, task_id);
    let mut lease = acquired(
        begin_task_mutation(
            &pool,
            TEST_WORKER_ID,
            TEST_CLAIM_ATTEMPT,
            task_id,
            "skill:config_edit:action:apply",
            "skill:config_edit:action:apply",
        )
        .expect("record intent"),
    );
    start_task_mutation_attempt(&pool, &mut lease).expect("start attempt");
    record_task_mutation_receipt(
        &pool,
        &lease,
        r#"{"status":"ok"}"#,
        Some(&serde_json::json!({"status_code": "mutation_applied"})),
    )
    .expect("record receipt");
    record_task_mutation_verification(
        &pool,
        &lease,
        &serde_json::json!({"schema_version": 1, "status": "inconclusive"}),
        false,
    )
    .expect("record pending verification");
    drop(pool);

    let pool = file_pool(&db_path);
    let resumed = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("resume pending verification");
    let BeginTaskMutationOutcome::ReplaySuppressed(record) = resumed else {
        panic!("pending verification must suppress original replay");
    };
    assert_eq!(record.phase, TaskMutationPhase::VerificationPending);
}

#[test]
fn applied_reconciliation_is_committable_without_original_action_replay() {
    let temp = TempDir::new();
    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    let task_id = "task-reconciled-applied";
    insert_active_task(&pool, task_id);
    let mut lease = acquired(
        begin_task_mutation(
            &pool,
            TEST_WORKER_ID,
            TEST_CLAIM_ATTEMPT,
            task_id,
            "skill:publish:action:create",
            "skill:publish:action:create",
        )
        .expect("record intent"),
    );
    start_task_mutation_attempt(&pool, &mut lease).expect("start attempt");
    mark_task_mutation_uncertain(&pool, &lease).expect("mark uncertain");

    let outcome = reconcile_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        &lease.record.fingerprint_hash,
        TaskMutationReconciliation::Applied,
        &serde_json::json!({
            "schema_version": 1,
            "disposition": "applied",
            "receipt_ref": "provider-operation-42"
        }),
    )
    .expect("reconcile applied");
    let ReconcileTaskMutationOutcome::Reconciled(reconciled_lease) = outcome else {
        panic!("expected reconciled lease");
    };
    drop(pool);

    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    let before_commit = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:publish:action:create",
        "skill:publish:action:create",
    )
    .expect("inspect reconciled mutation after restart");
    let BeginTaskMutationOutcome::ReplaySuppressed(record) = before_commit else {
        panic!("reconciled mutation must suppress replay before commit");
    };
    assert_eq!(record.phase, TaskMutationPhase::Reconciled);

    commit_task_mutation(&pool, &reconciled_lease).expect("commit reconciled mutation");

    let replay = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        task_id,
        "skill:publish:action:create",
        "skill:publish:action:create",
    )
    .expect("inspect reconciled commit");
    let BeginTaskMutationOutcome::ReplaySuppressed(record) = replay else {
        panic!("reconciled mutation must suppress replay");
    };
    assert_eq!(record.phase, TaskMutationPhase::Committed);
    assert_eq!(
        record
            .reconciliation
            .as_ref()
            .and_then(|value| value.get("receipt_ref"))
            .and_then(serde_json::Value::as_str),
        Some("provider-operation-42")
    );
}

#[test]
fn legacy_three_state_table_is_physically_migrated_to_v2() {
    let temp = TempDir::new();
    let pool = file_pool(&temp.0.join("tasks.sqlite"));
    insert_active_task(&pool, "task-legacy-ledger");
    {
        let db = pool.get().expect("get legacy db");
        db.execute_batch(
            "CREATE TABLE task_mutation_ledger (
                task_id TEXT NOT NULL,
                fingerprint_hash TEXT NOT NULL,
                action_ref TEXT NOT NULL,
                status TEXT NOT NULL,
                execution_token TEXT NOT NULL,
                lease_owner TEXT NOT NULL,
                claim_attempt INTEGER NOT NULL,
                outcome_hash TEXT,
                outcome_json TEXT,
                started_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                completed_at INTEGER,
                PRIMARY KEY (task_id, fingerprint_hash)
            );",
        )
        .expect("create legacy ledger");
        let fingerprint_hash = super::sha256_hex(b"skill:config_edit:action:apply");
        db.execute(
            "INSERT INTO task_mutation_ledger (
                task_id, fingerprint_hash, action_ref, status, execution_token,
                lease_owner, claim_attempt, outcome_hash, outcome_json,
                started_at, updated_at, completed_at
             ) VALUES (?1, ?2, ?3, 'completed', 'legacy-token', ?4, 1,
                       'receipt-hash', ?5, 1, 2, 2)",
            rusqlite::params![
                "task-legacy-ledger",
                fingerprint_hash,
                "skill:config_edit:action:apply",
                TEST_WORKER_ID,
                serde_json::json!({
                    "structured_extra": {"status_code": "legacy_completed"}
                })
                .to_string()
            ],
        )
        .expect("insert legacy mutation");
    }

    let outcome = begin_task_mutation(
        &pool,
        TEST_WORKER_ID,
        TEST_CLAIM_ATTEMPT,
        "task-legacy-ledger",
        "skill:config_edit:action:apply",
        "skill:config_edit:action:apply",
    )
    .expect("migrate legacy mutation");
    let BeginTaskMutationOutcome::ReplaySuppressed(record) = outcome else {
        panic!("legacy completion must remain replay-suppressed");
    };
    assert_eq!(record.phase, TaskMutationPhase::Committed);

    let db = pool.get().expect("get migrated db");
    let columns = {
        let mut statement = db
            .prepare("PRAGMA table_info(task_mutation_ledger)")
            .expect("prepare table info");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect columns")
    };
    assert!(columns.iter().any(|column| column == "phase"));
    assert!(columns.iter().any(|column| column == "idempotency_key"));
    assert!(!columns.iter().any(|column| column == "status"));
}
