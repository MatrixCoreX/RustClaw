use rusqlite::Connection;
use serde_json::json;

use super::{
    insert_scheduled_run_enqueued, scheduled_run_payload_metadata, scheduled_run_terminal_result,
    scheduled_run_thread_ref, scheduled_run_triage_from_machine, update_scheduled_run_terminal,
    ScheduledRunEnqueued, ScheduledRunTriage,
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
