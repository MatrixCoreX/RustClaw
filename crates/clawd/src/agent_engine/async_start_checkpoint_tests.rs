use serde_json::json;

use super::{
    build_pending_async_job_checkpoint_progress_payload, pending_async_job_ref_from_extra,
    pending_async_job_visible_reply_from_progress_payload,
};
use crate::agent_engine::LoopState;
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::task_lifecycle::{CheckpointBudgetCounters, ResumeEntrypoint};

fn test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "task-async-start".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn test_budget(loop_state: &LoopState, step: u32) -> CheckpointBudgetCounters {
    CheckpointBudgetCounters {
        round: u32::try_from(loop_state.round_no).unwrap_or(u32::MAX),
        step,
        llm_calls: 0,
        tool_calls: u32::try_from(loop_state.tool_calls_total).unwrap_or(u32::MAX),
        elapsed_ms: 0,
        llm_elapsed_ms: 0,
        tool_elapsed_ms: 0,
    }
}

#[test]
fn pending_async_job_extra_builds_machine_job_ref() {
    let extra = json!({
        "pending_async_job": {
            "job_id": "job-1",
            "status": "accepted",
            "poll_after_seconds": 30,
            "expires_at": 2000,
            "cancel_ref": "cancel:job-1",
            "message_key": "clawd.task.async_job_pending"
        }
    });

    let job = pending_async_job_ref_from_extra(Some(&extra))
        .expect("parse")
        .expect("job");

    assert_eq!(job.job_id, "job-1");
    assert_eq!(job.poll_after_seconds, 30);
    assert_eq!(job.expires_at, 2000);
    assert_eq!(job.cancel_ref, "cancel:job-1");
    assert_eq!(job.message_key, "clawd.task.async_job_pending");
}

#[test]
fn pending_async_job_extra_rejects_missing_machine_fields() {
    let extra = json!({
        "pending_async_job": {
            "job_id": "job-1",
            "status": "running"
        }
    });

    let err = pending_async_job_ref_from_extra(Some(&extra)).expect_err("invalid");

    assert!(err.contains("missing_required_fields"));
    assert!(err.contains("poll_after_seconds"));
    assert!(err.contains("expires_at"));
    assert!(err.contains("cancel_ref"));
    assert!(err.contains("message_key"));
}

#[test]
fn pending_async_job_checkpoint_uses_poll_resume_entrypoint() {
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.total_steps_executed = 3;
    loop_state.tool_calls_total = 2;
    loop_state
        .successful_action_fingerprints
        .insert("skill:video_basic:action:start_generation".to_string(), 1);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "video_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("{\"status\":\"accepted\"}".to_string()),
        error: None,
        started_at: 10,
        finished_at: 11,
    });
    let job = pending_async_job_ref_from_extra(Some(&json!({
        "type": "pending_async_job",
        "job_id": "job-2",
        "status": "running",
        "poll_after_seconds": 45,
        "expires_at": 3000,
        "cancel_ref": "cancel:job-2",
        "message_key": "clawd.task.async_job_pending"
    })))
    .expect("parse")
    .expect("job");

    let payload = build_pending_async_job_checkpoint_progress_payload(
        &test_task(),
        &loop_state,
        "video_basic",
        3,
        1,
        &job,
        None,
        1000,
        test_budget(&loop_state, 3),
    );

    assert_eq!(payload["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        payload["task_lifecycle"]["source"],
        "async_job_start_adapter"
    );
    assert_eq!(payload["task_lifecycle"]["next_check_after"], 1045);
    assert_eq!(payload["task_lifecycle"]["poll_ref"], "job-2");
    assert_eq!(
        payload["task_checkpoint"]["resume_entrypoint"],
        serde_json::to_value(ResumeEntrypoint::PollAsyncJob).expect("resume entrypoint")
    );
    assert_eq!(
        payload["task_checkpoint"]["pending_async_job"]["job_id"],
        "job-2"
    );
    assert_eq!(
        payload["task_checkpoint"]["completed_side_effect_refs"][0],
        "skill:video_basic:action:start_generation"
    );
}

#[test]
fn pending_async_job_visible_reply_carries_checkpoint_markers() {
    let loop_state = LoopState::new(4);
    let job = pending_async_job_ref_from_extra(Some(&json!({
        "pending_async_job": {
            "job_id": "job-visible",
            "status": "accepted",
            "poll_after_seconds": 12,
            "expires_at": 3000,
            "cancel_ref": "cancel:job-visible",
            "message_key": "clawd.task.async_job_pending"
        }
    })))
    .expect("parse")
    .expect("job");
    let payload = build_pending_async_job_checkpoint_progress_payload(
        &test_task(),
        &loop_state,
        "run_cmd",
        1,
        1,
        &job,
        None,
        1000,
        test_budget(&loop_state, 1),
    );

    let reply = pending_async_job_visible_reply_from_progress_payload(&payload)
        .expect("visible machine reply");
    let reply_json: serde_json::Value = serde_json::from_str(&reply).expect("reply json");

    assert_eq!(reply_json["output_format"], "machine_json");
    assert_eq!(reply_json["status"], "accepted");
    assert_eq!(reply_json["poll_ref"], "job-visible");
    assert_eq!(reply_json["next_check_after"], 1012);
    assert!(reply_json["checkpoint_id"]
        .as_str()
        .is_some_and(|value| value.starts_with("agent-loop:task-async-start:")));
}

#[test]
fn pending_async_job_checkpoint_persists_skill_poll_adapter() {
    let loop_state = LoopState::new(4);
    let extra = json!({
        "pending_async_job": {
            "job_id": "provider:video_generate:minimax:task-1",
            "status": "accepted",
            "poll_after_seconds": 30,
            "expires_at": 3000,
            "cancel_ref": "provider:video_generate:minimax:task-1",
            "message_key": "clawd.task.async_job_pending",
            "poll_adapter": {
                "kind": "skill_poll",
                "skill_name": "video_generate",
                "args": {
                    "action": "poll",
                    "task_id": "task-1",
                    "vendor": "minimax"
                }
            }
        }
    });
    let job = pending_async_job_ref_from_extra(Some(&extra))
        .expect("parse")
        .expect("job");
    let poll_adapter = super::pending_async_job_poll_adapter_from_extra(Some(&extra))
        .expect("parse adapter")
        .expect("adapter");

    let payload = build_pending_async_job_checkpoint_progress_payload(
        &test_task(),
        &loop_state,
        "video_generate",
        1,
        1,
        &job,
        Some(&poll_adapter),
        1000,
        test_budget(&loop_state, 1),
    );

    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["async_poll_adapter"]["kind"],
        "skill_poll"
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["async_poll_adapter"]["skill_name"],
        "video_generate"
    );
    assert_eq!(
        payload["task_lifecycle"]["async_timeout_policy"]["adapter_kind"],
        "skill_poll"
    );
    assert_eq!(
        payload["task_lifecycle"]["async_timeout_policy"]["policy_source"],
        "async_job_contract"
    );
    assert!(
        payload["task_checkpoint"]["boundary_context"]["async_poll_adapter"]
            .get("text")
            .is_none()
    );
    assert!(
        payload["task_checkpoint"]["boundary_context"]["async_poll_adapter"]
            .get("error_text")
            .is_none()
    );
}
