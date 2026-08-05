use rusqlite::Connection;

use super::*;

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    db
}

fn fixture_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("fixture-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn fixture_settings() -> super::super::settings::MemoryEffectiveSettings {
    use super::super::settings::{
        ExternalContextPolicy, MemoryEffectiveSettings, MemoryRequestedSettings, MemorySettingMode,
        MemorySettingScope,
    };
    MemoryEffectiveSettings {
        schema_version: 1,
        scope: MemorySettingScope::Principal,
        target_principal_id: "principal-fixture".to_string(),
        conversation_id: None,
        requested: MemoryRequestedSettings {
            use_mode: MemorySettingMode::Enabled,
            generate_mode: MemorySettingMode::Enabled,
            external_context_policy: ExternalContextPolicy::Exclude,
        },
        use_memory: true,
        generate_memory: true,
        external_context_policy: ExternalContextPolicy::Exclude,
        use_source: "fixture".to_string(),
        generate_source: "fixture".to_string(),
        external_context_source: "fixture".to_string(),
        managed_deny_reason: None,
        revision: 1,
        policy_digest: "policy-fixture".to_string(),
        restart_required: false,
    }
}

#[test]
fn durable_job_schema_is_idempotent_and_has_required_constraints() {
    let db = setup_db();
    ensure_memory_job_schema(&db).expect("first migration");
    ensure_memory_job_schema(&db).expect("idempotent migration");

    for table in [
        "memory_source_events",
        "memory_jobs",
        "memory_raw_candidates",
        "memory_evidence",
        "memory_retention_ledger",
        "memory_storage_pressure",
        "memory_principal_quotas",
    ] {
        assert!(table_exists(&db, table).expect("inspect table"), "{table}");
    }
    let pressure: (String, i64) = db
        .query_row(
            "SELECT state, revision FROM memory_storage_pressure WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("pressure seed");
    assert_eq!(pressure, ("normal".to_string(), 1));
}

#[test]
fn cancellation_is_principal_and_scope_bounded() {
    let db = setup_db();
    ensure_memory_job_schema(&db).expect("job schema");
    for (job_id, principal_id, scope_ref) in [
        ("job-a", "principal-a", "conversation-a"),
        ("job-b", "principal-a", "conversation-b"),
        ("job-c", "principal-b", "conversation-a"),
    ] {
        db.execute(
            "INSERT INTO memory_jobs(
                job_id, job_kind, principal_id, scope_kind, scope_ref, source_digest,
                eligibility_json, settings_revision, policy_digest, provider_name,
                provider_type, model_name, model_capability_digest, status,
                not_before_ts, created_at_ts, updated_at_ts
             ) VALUES (?1, 'extract', ?2, 'conversation', ?3, 'digest', '{}', 1,
                       'policy', 'provider', 'provider_type', 'model', 'capability',
                       'queued', 0, 0, 0)",
            rusqlite::params![job_id, principal_id, scope_ref],
        )
        .expect("insert fixture job");
    }

    assert_eq!(
        request_cancel_for_scope(&db, "principal-a", Some("conversation-a")).expect("cancel scope"),
        1
    );
    let cancelled = db
        .prepare("SELECT job_id FROM memory_jobs WHERE cancel_requested = 1 ORDER BY job_id")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    assert_eq!(cancelled, vec!["job-a"]);
}

#[test]
fn source_events_and_outbox_jobs_share_the_callers_transaction() {
    let mut db = setup_db();
    ensure_memory_job_schema(&db).expect("job schema");
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = fixture_task("task-atomic");
    let settings = fixture_settings();
    let eligibility =
        super::super::eligibility::build_turn_eligibility(&task, &serde_json::json!({}), &settings);
    let transaction = db.transaction().expect("transaction");
    for (id, role) in [(1_i64, "user"), (2_i64, "assistant")] {
        transaction
            .execute(
                "INSERT INTO memories(
                    id, memory_id, user_id, chat_id, user_key, principal_id, scope_kind,
                    scope_ref, channel, role, content, created_at, created_at_ts
                 ) VALUES (?1, ?2, 1, 2, 'fixture-key', 'principal-fixture', 'principal',
                           'principal-fixture', 'ui', ?3, ?4, '1', 1)",
                rusqlite::params![id, format!("memory-{id}"), role, format!("content-{id}")],
            )
            .expect("source row");
    }
    let jobs = enqueue_prepared_turn_jobs(
        &state,
        &task,
        &settings,
        &eligibility,
        "principal",
        "principal-fixture",
        Some(1),
        Some(2),
        true,
        &transaction,
    )
    .expect("enqueue in transaction");
    assert!(!jobs.is_empty());
    transaction.rollback().expect("rollback");
    for table in ["memories", "memory_source_events", "memory_jobs"] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count after rollback");
        assert_eq!(count, 0, "{table}");
    }
}

#[test]
fn claims_are_single_owner_recover_expired_leases_and_favor_unserved_principals() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("state fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    ensure_memory_job_schema(&db).expect("job schema");
    let now = crate::now_ts_u64() as i64;
    for (job_id, principal, created_at) in [
        ("job-a1", "principal-a", now - 3),
        ("job-a2", "principal-a", now - 2),
        ("job-b1", "principal-b", now - 1),
    ] {
        db.execute(
            "INSERT INTO memory_jobs(
                job_id, job_kind, principal_id, scope_kind, scope_ref, source_digest,
                eligibility_json, settings_revision, policy_digest, provider_name,
                provider_type, model_name, model_capability_digest, status,
                not_before_ts, created_at_ts, updated_at_ts
             ) VALUES (?1, 'extract', ?2, 'principal', ?2, 'digest', '{}', 1,
                       'policy', 'provider', 'type', 'model', 'capability', 'queued',
                       ?3, ?4, ?4)",
            rusqlite::params![job_id, principal, now - 10, created_at],
        )
        .expect("insert job");
    }
    drop(db);
    let first = claim_next_job(&state, "worker-a")
        .expect("first claim")
        .expect("first job");
    assert_eq!(first.job_id, "job-a1");
    let second = claim_next_job(&state, "worker-b")
        .expect("second claim")
        .expect("second job");
    assert_eq!(second.job_id, "job-b1");
    let db = state.core.db.get().expect("state db");
    db.execute(
        "UPDATE memory_jobs SET lease_expires_at_ts = ?1 WHERE job_id = 'job-a1'",
        [now - 1],
    )
    .expect("expire lease");
    drop(db);
    let remaining = claim_next_job(&state, "worker-c")
        .expect("remaining claim")
        .expect("remaining job");
    assert_eq!(remaining.job_id, "job-a2");
    let recovered = claim_next_job(&state, "worker-d")
        .expect("recovery claim")
        .expect("recovered job");
    assert_eq!(recovered.job_id, "job-a1");
    assert_eq!(recovered.attempt, 2);
}

#[test]
fn active_worker_heartbeat_renews_lease_and_respects_cancellation_fence() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("state fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    ensure_memory_job_schema(&db).expect("job schema");
    let now = crate::now_ts_u64() as i64;
    db.execute(
        "INSERT INTO memory_jobs(
            job_id, job_kind, principal_id, scope_kind, scope_ref, source_digest,
            eligibility_json, settings_revision, policy_digest, provider_name,
            provider_type, model_name, model_capability_digest, status, lease_owner,
            lease_expires_at_ts, not_before_ts, created_at_ts, updated_at_ts
         ) VALUES ('heartbeat-job', 'extract', 'principal-a', 'principal',
                   'principal-a', 'digest', '{}', 1, 'policy', 'provider', 'type',
                   'model', 'capability', 'running', 'worker-a', ?1, ?1, ?1, ?1)",
        [now + 1],
    )
    .expect("running job");
    drop(db);

    renew_job_lease(&state, "heartbeat-job", "worker-a").expect("renew lease");
    let db = state.core.db.get().unwrap();
    let renewed: i64 = db
        .query_row(
            "SELECT lease_expires_at_ts FROM memory_jobs WHERE job_id = 'heartbeat-job'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(renewed >= now + state.policy.memory.background_lease_seconds as i64);
    db.execute(
        "UPDATE memory_jobs SET cancel_requested = 1 WHERE job_id = 'heartbeat-job'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(renew_job_lease(&state, "heartbeat-job", "worker-a").is_err());
}
