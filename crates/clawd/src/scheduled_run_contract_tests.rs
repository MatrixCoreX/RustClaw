use rusqlite::Connection;
use serde_json::json;

use super::{
    insert_scheduled_run_enqueued, list_scheduled_run_history, scheduled_run_payload_metadata,
    scheduled_run_policy_metadata, scheduled_run_terminal_result, scheduled_run_thread_ref,
    scheduled_run_thread_resume_metadata, scheduled_run_triage_from_machine,
    update_scheduled_run_terminal, ScheduledRunEnqueued, ScheduledRunTriage,
};

#[test]
fn scheduled_run_metadata_binds_job_run_and_thread_refs() {
    let metadata = scheduled_run_payload_metadata("job_abc123", "run_001");
    let keys: std::collections::HashSet<_> = metadata.iter().map(|(key, _)| key.as_str()).collect();

    assert!(keys.contains("automation_run_id"));
    assert!(keys.contains("automation_thread_ref"));
    assert!(keys.contains("thread_ref"));
    assert!(keys.contains("scheduled_run_schema_version"));
    assert!(keys.contains("scheduled_run_ref"));
    assert_eq!(
        scheduled_run_thread_ref("job_abc123"),
        "scheduled_job:job_abc123"
    );
}

#[test]
fn scheduled_run_policy_metadata_sanitizes_profile_and_policy_fields() {
    let metadata = scheduled_run_policy_metadata(
        " local_worktree ",
        r#"{
            "filesystem_write": true,
            "network_access": false,
            "risk_level": "medium",
            "notes": "visible policy prose"
        }"#,
    );
    let map: serde_json::Map<String, serde_json::Value> = metadata.into_iter().collect();

    assert_eq!(
        map.get("automation_isolation_profile")
            .and_then(serde_json::Value::as_str),
        Some("local_worktree")
    );
    let policy = map
        .get("automation_permission_policy")
        .expect("permission policy");
    assert_eq!(policy["filesystem_write"], true);
    assert_eq!(policy["network_access"], false);
    assert_eq!(policy["risk_level"], "medium");
    assert!(policy.get("notes").is_none());

    let fallback = scheduled_run_policy_metadata("bad profile", "not-json");
    let fallback_map: serde_json::Map<String, serde_json::Value> = fallback.into_iter().collect();
    assert_eq!(
        fallback_map
            .get("automation_isolation_profile")
            .and_then(serde_json::Value::as_str),
        Some("local_current_workspace")
    );
    assert_eq!(
        fallback_map
            .get("automation_permission_policy")
            .and_then(serde_json::Value::as_object)
            .map(|value| value.len()),
        Some(0)
    );
}

#[test]
fn scheduled_run_thread_resume_metadata_binds_prior_task_when_enabled() {
    let metadata = scheduled_run_thread_resume_metadata(true, Some(" task-prev "));
    let map: serde_json::Map<String, serde_json::Value> = metadata.into_iter().collect();

    assert_eq!(
        map.get("thread_resume")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        map.get("thread_resume_mode")
            .and_then(serde_json::Value::as_str),
        Some("prior_scheduled_task")
    );
    assert_eq!(
        map.get("thread_resume_task_id")
            .and_then(serde_json::Value::as_str),
        Some("task-prev")
    );
    assert_eq!(
        map.get("source_task_id")
            .and_then(serde_json::Value::as_str),
        Some("task-prev")
    );

    let disabled = scheduled_run_thread_resume_metadata(false, Some("task-prev"));
    let disabled_map: serde_json::Map<String, serde_json::Value> = disabled.into_iter().collect();
    assert_eq!(
        disabled_map
            .get("thread_resume")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(disabled_map.get("thread_resume_task_id").is_none());
}

#[test]
fn scheduled_run_triage_uses_machine_status_and_findings_only() {
    assert_eq!(
        scheduled_run_triage_from_machine("failed", None),
        ScheduledRunTriage::Failed
    );
    assert_eq!(
        scheduled_run_triage_from_machine("canceled", None),
        ScheduledRunTriage::Cancelled
    );
    assert_eq!(
        scheduled_run_triage_from_machine(
            "succeeded",
            Some(&json!({"task_lifecycle": {"state": "needs_user"}}))
        ),
        ScheduledRunTriage::NeedsUser
    );
    assert_eq!(
        scheduled_run_triage_from_machine(
            "succeeded",
            Some(&json!({
                "finding_refs": ["finding:1"],
                "text": "ignored visible result"
            }))
        ),
        ScheduledRunTriage::Findings
    );
    assert_eq!(
        scheduled_run_triage_from_machine("succeeded", Some(&json!({}))),
        ScheduledRunTriage::NoFindings
    );
}

#[test]
fn scheduled_run_terminal_result_excludes_visible_text_fields() {
    let result = scheduled_run_terminal_result(
        false,
        &json!({
            "automation_ref": "job_abc123",
            "automation_kind": "scheduled_job",
            "automation_run_id": "run_001",
            "thread_ref": "scheduled_job:job_abc123",
            "error_code": "provider_unavailable",
            "finding_refs": ["finding:1"],
            "policy_decision": {
                "decision": "denied",
                "reason_code": "credential_access_blocked",
                "explanation": "visible policy prose"
            },
            "provider_status": {
                "provider": "minimax",
                "status": "failed",
                "error_text": "visible provider prose"
            },
            "text": "visible text must not be copied",
            "error_text": "visible error must not be copied"
        }),
        Some(&json!({"delivered": false, "error_code": "channel_send_failed"})),
    );

    assert_eq!(result["task_success"], false);
    assert_eq!(result["automation_ref"], "job_abc123");
    assert_eq!(result["error_code"], "provider_unavailable");
    assert_eq!(result["finding_refs"][0], "finding:1");
    assert_eq!(
        result["policy_decision"]["reason_code"],
        "credential_access_blocked"
    );
    assert_eq!(result["provider_status"]["provider"], "minimax");
    assert!(result["policy_decision"].get("explanation").is_none());
    assert!(result["provider_status"].get("error_text").is_none());
    assert!(result.get("text").is_none());
    assert!(result.get("error_text").is_none());
}

#[test]
fn scheduled_run_history_insert_and_terminal_update_are_machine_rows() {
    let db = Connection::open_in_memory().expect("open test db");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    insert_test_scheduled_job(&db, "job_abc123", 7, 11);

    insert_scheduled_run_enqueued(
        &db,
        &ScheduledRunEnqueued {
            run_id: "run_001",
            job_id: "job_abc123",
            task_id: "task_001",
            thread_ref: "scheduled_job:job_abc123",
            started_at: "1000",
        },
    )
    .expect("insert run");

    let queued: String = db
        .query_row(
            "SELECT task_status FROM scheduled_job_runs WHERE run_id = 'run_001'",
            [],
            |row| row.get(0),
        )
        .expect("queued row");
    assert_eq!(queued, "queued");

    let result = json!({"finding_refs": ["finding:1"]});
    let affected =
        update_scheduled_run_terminal(&db, "job_abc123", "task_001", "succeeded", "1010", &result)
            .expect("terminal update");
    assert_eq!(affected, 1);

    let (status, triage): (String, String) = db
        .query_row(
            "SELECT task_status, triage_status FROM scheduled_job_runs WHERE run_id = 'run_001'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("updated row");
    assert_eq!(status, "succeeded");
    assert_eq!(triage, "findings");
}

#[test]
fn scheduled_run_history_listing_filters_owner_and_job() {
    let db = Connection::open_in_memory().expect("open test db");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    insert_test_scheduled_job(&db, "job_abc123", 7, 11);
    insert_test_scheduled_job(&db, "job_other", 7, 11);
    insert_test_scheduled_job(&db, "job_foreign", 8, 11);

    for (run_id, job_id, task_id, finding) in [
        ("run_001", "job_abc123", "task_001", "finding:1"),
        ("run_002", "job_other", "task_002", "finding:2"),
        ("run_003", "job_foreign", "task_003", "finding:3"),
    ] {
        insert_scheduled_run_enqueued(
            &db,
            &ScheduledRunEnqueued {
                run_id,
                job_id,
                task_id,
                thread_ref: &format!("scheduled_job:{job_id}"),
                started_at: "1000",
            },
        )
        .expect("insert run");
        let result = json!({"finding_refs": [finding], "text": "ignored visible text"});
        update_scheduled_run_terminal(&db, job_id, task_id, "succeeded", "1010", &result)
            .expect("terminal update");
    }

    let all = list_scheduled_run_history(&db, 7, 11, None, 10).expect("list all owner runs");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0]["task_status"], "succeeded");
    assert!(all[0].get("result").is_none());
    assert!(all[0]["result_summary"].get("text").is_none());
    assert_eq!(all[0]["finding_refs"][0], "finding:2");

    let filtered =
        list_scheduled_run_history(&db, 7, 11, Some("job_abc123"), 10).expect("list job runs");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0]["run_id"], "run_001");
    assert_eq!(filtered[0]["finding_refs"][0], "finding:1");
}

#[test]
fn scheduled_run_history_listing_exposes_policy_provider_machine_summary() {
    let db = Connection::open_in_memory().expect("open test db");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    insert_test_scheduled_job(&db, "job_abc123", 7, 11);
    insert_scheduled_run_enqueued(
        &db,
        &ScheduledRunEnqueued {
            run_id: "run_policy",
            job_id: "job_abc123",
            task_id: "task_policy",
            thread_ref: "scheduled_job:job_abc123",
            started_at: "1000",
        },
    )
    .expect("insert run");
    let result = json!({
        "schema_version": 1,
        "task_success": false,
        "error_code": "provider_rate_limited",
        "policy_decision": {
            "decision": "denied",
            "reason_code": "credential_access_blocked",
            "explanation": "visible policy prose"
        },
        "provider_status": {
            "provider": "minimax",
            "status": "failed",
            "error_text": "visible provider prose"
        },
        "notification": {
            "delivered": false,
            "runtime_channel": "ui",
            "error_code": "channel_send_failed"
        },
        "text": "visible result prose"
    });
    update_scheduled_run_terminal(&db, "job_abc123", "task_policy", "failed", "1010", &result)
        .expect("terminal update");

    let rows = list_scheduled_run_history(&db, 7, 11, Some("job_abc123"), 10).expect("list runs");
    let summary = &rows[0]["result_summary"];

    assert_eq!(rows[0]["triage_status"], "failed");
    assert_eq!(summary["error_code"], "provider_rate_limited");
    assert_eq!(summary["policy_decision"]["decision"], "denied");
    assert_eq!(
        summary["policy_decision"]["reason_code"],
        "credential_access_blocked"
    );
    assert_eq!(summary["provider_status"]["provider"], "minimax");
    assert_eq!(summary["provider_status"]["status"], "failed");
    assert_eq!(summary["notification"]["delivered"], false);
    assert_eq!(summary["notification"]["error_code"], "channel_send_failed");
    assert!(summary["policy_decision"].get("explanation").is_none());
    assert!(summary["provider_status"].get("error_text").is_none());
    assert!(summary.get("text").is_none());
    assert!(rows[0].get("result").is_none());
}

fn insert_test_scheduled_job(db: &Connection, job_id: &str, user_id: i64, chat_id: i64) {
    db.execute(
        "INSERT INTO scheduled_jobs (
            job_id, user_id, chat_id, channel, schedule_type, timezone,
            task_kind, task_payload_json, enabled, notify_on_success,
            notify_on_failure, next_run_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, 'ui', 'once', 'UTC', 'ask', '{}', 1, 1, 1, 1000, '1000', '1000')",
        rusqlite::params![job_id, user_id, chat_id],
    )
    .expect("insert scheduled job");
}
