use serde_json::json;

use super::{
    async_job_contract_json, async_job_protocol_hint_line, async_poll_adapter_result_matches_job,
    async_poll_adapter_status, ASYNC_POLL_ADAPTER_RESULT_KEY,
};

#[test]
fn async_job_protocol_hint_exposes_machine_contract() {
    let hint = async_job_protocol_hint_line();

    assert!(hint.contains("async_job_protocol=version:1"));
    assert!(hint.contains("phases:start|checkpoint|poll|observation|verify_finalize"));
    assert!(hint.contains("resume_entrypoint:poll_async_job"));
    assert!(hint.contains("checkpoint_states:waiting|background"));
    assert!(hint.contains("adapter_statuses:accepted|running|succeeded|failed|expired"));
    assert!(hint.contains(
        "required_job_fields:job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key"
    ));
    assert!(hint.contains("adapter_result_key:async_poll_adapter_result"));
    assert!(hint.contains("user_text_fields_forbidden:text|error_text"));
}

#[test]
fn async_job_contract_json_uses_only_machine_fields() {
    let contract = async_job_contract_json();

    assert_eq!(contract["schema_version"], 1);
    assert_eq!(contract["resume_entrypoint"], "poll_async_job");
    assert_eq!(
        contract["adapter_result_key"],
        ASYNC_POLL_ADAPTER_RESULT_KEY
    );
    assert_eq!(contract["phases"][0], "start");
    assert_eq!(contract["adapter_statuses"][2], "succeeded");
    assert_eq!(contract["forbidden_user_text_fields"][0], "text");
}

#[test]
fn adapter_result_contract_rejects_user_text_and_job_mismatch() {
    let accepted = json!({
        "job_id": "job-1",
        "status": "accepted",
        "poll_after_seconds": 10,
        "expires_at": 2000
    });
    assert_eq!(async_poll_adapter_status(&accepted), Some("accepted"));
    assert!(async_poll_adapter_result_matches_job(&accepted, "job-1"));

    let text_leak = json!({
        "job_id": "job-1",
        "status": "running",
        "text": "leak"
    });
    assert!(!async_poll_adapter_result_matches_job(&text_leak, "job-1"));

    let mismatch = json!({
        "job_id": "job-2",
        "status": "succeeded",
        "final_result_json": {"status": "ok"}
    });
    assert!(!async_poll_adapter_result_matches_job(&mismatch, "job-1"));
}
