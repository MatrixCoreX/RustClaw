use serde_json::json;

use super::{
    async_job_contract_json, async_job_protocol_hint_line, async_poll_adapter_kind_supported,
    async_poll_adapter_result_matches_job, async_poll_adapter_status,
    parse_pending_async_job_poll_adapter_from_extra, parse_pending_async_job_ref_from_extra,
    pending_async_job_contract_summary, pending_async_job_timeout_policy,
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
        "canonical_job_fields:job_id|provider|status|poll_after_ms|cancel_token|result_ref|error_code|retryable"
    ));
    assert!(hint.contains("legacy_compat_fields:poll_after_seconds|cancel_ref"));
    assert!(hint.contains(
        "poll_adapter_kinds:skill_poll|local_process_poll|http_job_poll|mcp_job_poll|media_job_poll|browser_job_poll|remote_job_poll"
    ));
    assert!(hint.contains("adapter_result_key:async_poll_adapter_result"));
    assert!(hint.contains("adapter_result_fields:job_id|status|poll_after_seconds|poll_after_ms|expires_at|result_ref|final_result_json|failure_result_json|cancellation_result_json|error_code|message_key|retryable"));
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
    assert_eq!(contract["canonical_job_fields"][1], "provider");
    assert_eq!(contract["canonical_job_fields"][3], "poll_after_ms");
    assert_eq!(contract["legacy_compat_fields"][1], "cancel_ref");
    assert_eq!(contract["adapter_result_fields"][3], "poll_after_ms");
    assert_eq!(contract["adapter_result_fields"][5], "result_ref");
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
    assert_eq!(policy["max_runtime_deadline_ts"], 1_900);
    assert_eq!(policy["deadline_ts"], 1_900);
    assert_eq!(policy["effective_deadline_ts"], 1_900);
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
    assert_eq!(policy["effective_deadline_ts"], 900);
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
fn pending_async_job_contract_summary_validates_required_machine_fields() {
    let extra = json!({
        "pending_async_job": {
            "job_id": "provider:video_generate:minimax:task-1",
            "status": "accepted",
            "poll_after_seconds": 5,
            "expires_at": 2_000,
            "cancel_ref": "provider:video_generate:minimax:task-1",
            "message_key": "clawd.task.async_job_pending",
            "poll_adapter": {
                "kind": "media_job_poll",
                "skill_name": "video_generate",
                "args": {"action": "poll", "task_id": "task-1"}
            }
        }
    });

    let summary = pending_async_job_contract_summary(Some(&extra), "test")
        .expect("valid contract")
        .expect("summary");
    let parsed = parse_pending_async_job_ref_from_extra(Some(&extra), "test")
        .expect("valid pending job")
        .expect("pending job");

    assert_eq!(summary["status"], "valid");
    assert_eq!(summary["job_id"], "provider:video_generate:minimax:task-1");
    assert_eq!(summary["provider"], "minimax");
    assert_eq!(summary["job_status"], "accepted");
    assert_eq!(summary["poll_after_ms"], 5_000);
    assert_eq!(summary["poll_adapter_kind"], "media_job_poll");
    assert_eq!(summary["poll_adapter_supported"], true);
    assert_eq!(summary["cancel_ref_present"], true);
    assert_eq!(summary["cancel_token_present"], true);
    assert_eq!(summary["retryable"], true);
    assert_eq!(summary["forbidden_user_text_fields_absent"], true);
    assert_eq!(summary["canonical_job_fields"][0], "job_id");
    assert_eq!(parsed.job_id, "provider:video_generate:minimax:task-1");
}

#[test]
fn pending_async_job_contract_accepts_canonical_alias_fields() {
    let extra = json!({
        "provider": "minimax",
        "pending_async_job": {
            "job_id": "provider:video_generate:minimax:task-1",
            "provider": "minimax",
            "status": "running",
            "poll_after_ms": 1500,
            "expires_at": 2_000,
            "cancel_token": "provider:video_generate:minimax:task-1",
            "result_ref": "provider:video_generate:minimax:task-1",
            "message_key": "clawd.task.async_job_pending",
            "retryable": true,
            "poll_adapter": {
                "kind": "media_job_poll",
                "skill_name": "video_generate",
                "args": {"action": "poll", "task_id": "task-1"}
            }
        }
    });

    let summary = pending_async_job_contract_summary(Some(&extra), "test")
        .expect("valid canonical contract")
        .expect("summary");
    let parsed = parse_pending_async_job_ref_from_extra(Some(&extra), "test")
        .expect("valid pending job")
        .expect("pending job");

    assert_eq!(summary["provider"], "minimax");
    assert_eq!(summary["poll_after_seconds"], 2);
    assert_eq!(summary["poll_after_ms"], 1500);
    assert_eq!(summary["cancel_ref_present"], false);
    assert_eq!(summary["cancel_token_present"], true);
    assert_eq!(
        summary["result_ref"],
        "provider:video_generate:minimax:task-1"
    );
    assert_eq!(summary["retryable"], true);
    assert_eq!(parsed.poll_after_seconds, 2);
    assert_eq!(parsed.cancel_ref, "provider:video_generate:minimax:task-1");
}

#[test]
fn pending_async_job_contract_rejects_missing_required_fields() {
    let extra = json!({
        "pending_async_job": {
            "job_id": "provider:video_generate:minimax:task-1",
            "status": "accepted",
            "poll_after_seconds": 5,
            "expires_at": 2_000,
            "message_key": "clawd.task.async_job_pending"
        }
    });

    let err = pending_async_job_contract_summary(Some(&extra), "test")
        .expect_err("missing cancel ref should fail");

    assert!(err.contains("missing_required_fields=cancel_ref"));
}

#[test]
fn pending_async_job_contract_rejects_user_text_fields() {
    let extra = json!({
        "pending_async_job": {
            "job_id": "job-1",
            "status": "accepted",
            "poll_after_seconds": 5,
            "expires_at": 2_000,
            "cancel_ref": "cancel:job-1",
            "message_key": "clawd.task.async_job_pending",
            "text": "visible fallback must not control async handoff"
        }
    });

    let err = pending_async_job_contract_summary(Some(&extra), "test")
        .expect_err("text field should fail");

    assert!(err.contains("user_text_fields_forbidden"));
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
