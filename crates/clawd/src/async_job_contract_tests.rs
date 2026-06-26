use serde_json::json;

use super::{
    async_job_contract_json, async_job_protocol_hint_line, async_poll_adapter_kind_supported,
    async_poll_adapter_result_matches_job, async_poll_adapter_status,
    parse_pending_async_job_poll_adapter_from_extra, pending_async_job_timeout_policy,
    skill_runner_poll_adapter_kind_supported, ASYNC_POLL_ADAPTER_RESULT_KEY,
};

#[test]
fn async_job_protocol_hint_exposes_machine_contract() {
    let hint = async_job_protocol_hint_line();

    assert!(hint.contains("async_job_protocol=version:1"));
    assert!(hint.contains("phases:start|checkpoint|poll|observation|verify_finalize"));
    assert!(hint.contains("resume_entrypoint:poll_async_job"));
    assert!(hint.contains("checkpoint_states:waiting|background"));
    assert!(hint.contains("adapter_statuses:accepted|running|succeeded|failed|expired|cancelled"));
    assert!(hint.contains(
        "required_job_fields:job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key"
    ));
    assert!(hint.contains(
        "poll_adapter_kinds:skill_poll|local_process_poll|http_job_poll|mcp_job_poll|media_job_poll|browser_job_poll|remote_job_poll"
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
    assert_eq!(contract["adapter_statuses"][5], "cancelled");
    assert_eq!(contract["poll_adapter_kinds"][1], "local_process_poll");
    assert_eq!(contract["poll_adapter_kinds"][4], "media_job_poll");
    assert_eq!(contract["timeout_policy_fields"][0], "adapter_kind");
    assert_eq!(contract["forbidden_user_text_fields"][0], "text");
}

#[test]
fn pending_async_job_timeout_policy_uses_adapter_family() {
    let job = crate::task_lifecycle::AsyncJobRef {
        job_id: "provider:video_generate:minimax:task-1".to_string(),
        status: crate::task_lifecycle::AsyncJobStatus::Accepted,
        poll_after_seconds: 5,
        expires_at: 1_900,
        cancel_ref: "provider:video_generate:minimax:task-1".to_string(),
        message_key: "clawd.task.async_job_pending".to_string(),
    };
    let adapter = json!({"kind": "media_job_poll"});

    let policy = pending_async_job_timeout_policy(Some(&adapter), &job, 1_000);

    assert_eq!(policy["policy_source"], "async_job_contract");
    assert_eq!(policy["adapter_kind"], "media_job_poll");
    assert_eq!(policy["max_runtime_seconds"], 900);
    assert_eq!(policy["deadline_ts"], 1_900);
    assert_eq!(policy["remaining_seconds"], 900);
    assert!(policy.get("text").is_none());
    assert!(policy.get("error_text").is_none());
}

#[test]
fn pending_async_job_timeout_policy_infers_local_process_without_adapter() {
    let job = crate::task_lifecycle::AsyncJobRef {
        job_id: "local_process:job-1".to_string(),
        status: crate::task_lifecycle::AsyncJobStatus::Running,
        poll_after_seconds: 5,
        expires_at: 900,
        cancel_ref: "local_process:/tmp/job-1".to_string(),
        message_key: "clawd.task.async_job_pending".to_string(),
    };

    let policy = pending_async_job_timeout_policy(None, &job, 1_000);

    assert_eq!(policy["adapter_kind"], "local_process_poll");
    assert_eq!(policy["max_runtime_seconds"], 3600);
    assert_eq!(policy["remaining_seconds"], 0);
}

#[test]
fn pending_async_job_poll_adapter_accepts_declared_adapter_kinds() {
    for adapter_kind in [
        "skill_poll",
        "http_job_poll",
        "mcp_job_poll",
        "media_job_poll",
        "browser_job_poll",
        "remote_job_poll",
    ] {
        assert!(async_poll_adapter_kind_supported(adapter_kind));
        assert!(skill_runner_poll_adapter_kind_supported(adapter_kind));
        let extra = json!({
            "pending_async_job": {
                "job_id": "job-1",
                "status": "accepted",
                "poll_after_seconds": 2,
                "expires_at": 2000,
                "cancel_ref": "cancel:job-1",
                "message_key": "clawd.task.async_job_pending",
                "poll_adapter": {
                    "kind": adapter_kind,
                    "skill_name": "video_generate",
                    "args": {"action": "poll", "task_id": "task-1"}
                }
            }
        });

        let parsed = parse_pending_async_job_poll_adapter_from_extra(Some(&extra), "test")
            .expect("valid adapter")
            .expect("adapter");
        assert_eq!(parsed["kind"], adapter_kind);
        assert!(parsed.get("text").is_none());
        assert!(parsed.get("error_text").is_none());
    }
}

#[test]
fn pending_async_job_poll_adapter_accepts_local_process_kind_without_skill_name() {
    assert!(async_poll_adapter_kind_supported("local_process_poll"));
    assert!(!skill_runner_poll_adapter_kind_supported(
        "local_process_poll"
    ));
    let extra = json!({
        "pending_async_job": {
            "job_id": "local_process:/tmp/job-1",
            "status": "accepted",
            "poll_after_seconds": 2,
            "expires_at": 2000,
            "cancel_ref": "local_process:/tmp/job-1",
            "message_key": "clawd.task.async_job_pending",
            "poll_adapter": {
                "kind": "local_process_poll"
            }
        }
    });

    let parsed = parse_pending_async_job_poll_adapter_from_extra(Some(&extra), "test")
        .expect("valid adapter")
        .expect("adapter");
    assert_eq!(parsed["kind"], "local_process_poll");
}

#[test]
fn pending_async_job_poll_adapter_rejects_unknown_kind() {
    let extra = json!({
        "pending_async_job": {
            "job_id": "job-1",
            "status": "accepted",
            "poll_after_seconds": 2,
            "expires_at": 2000,
            "cancel_ref": "cancel:job-1",
            "message_key": "clawd.task.async_job_pending",
            "poll_adapter": {
                "kind": "unknown_job_poll",
                "skill_name": "video_generate"
            }
        }
    });

    let err = parse_pending_async_job_poll_adapter_from_extra(Some(&extra), "test")
        .expect_err("invalid adapter");
    assert!(err.contains("unsupported_poll_adapter_kind"));
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
