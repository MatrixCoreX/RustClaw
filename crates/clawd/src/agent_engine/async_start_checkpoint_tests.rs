use serde_json::json;

use super::{
    build_pending_async_job_checkpoint_progress_payload, pending_async_job_ref_from_extra,
    pending_async_job_visible_reply_from_progress_payload,
};
use crate::agent_engine::LoopState;
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::task_lifecycle::{CheckpointBudgetCounters, ResumeEntrypoint};
use claw_core::capability_result::{
    CapabilityDeliveryIntent, CapabilityResultEnvelope, CapabilityResultStatus, Continuation,
    ContinuationKind,
};

fn test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
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
    let mut loop_state = LoopState::new();
    loop_state.task_budget_slice = Some(crate::task_budget_contract::TaskBudgetSlice::new(
        crate::task_budget_contract::TaskBudgetProfile::MultiStepWorkspace,
        30_000,
        crate::task_budget_contract::BudgetHardCeilings::default(),
    ));
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
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["task_budget_slice"]["profile"],
        "multi_step_workspace"
    );
    assert_eq!(
        payload["task_lifecycle"]["task_budget_slice"]["soft_slice_ms"],
        30_000
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["async_completion_policy"]["mode"],
        "direct_terminal"
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["async_completion_policy"]
            ["continuation_action_count"],
        0
    );
}

#[test]
fn pending_async_job_checkpoint_preserves_unexecuted_plan_tail() {
    let mut loop_state = LoopState::new();
    loop_state.active_verified_actions = vec![
        crate::AgentAction::CallCapability {
            capability: "system.run_command".to_string(),
            args: json!({"command": "sleep 1", "async_start": true}),
        },
        crate::AgentAction::CallCapability {
            capability: "system.health_check".to_string(),
            args: json!({}),
        },
    ];
    let job = pending_async_job_ref_from_extra(Some(&json!({
        "pending_async_job": {
            "job_id": "job-with-tail",
            "status": "accepted",
            "poll_after_seconds": 2,
            "expires_at": 3000,
            "cancel_ref": "cancel:job-with-tail",
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
    let policy = &payload["task_checkpoint"]["boundary_context"]["async_completion_policy"];
    assert_eq!(policy["mode"], "continue_planning");
    assert_eq!(policy["continuation_action_count"], 1);
    assert_eq!(
        policy["continuation_actions"][0]["capability"],
        "system.health_check"
    );
}

#[test]
fn pending_model_synthesis_job_resumes_planner_without_verified_action_tail() {
    let mut loop_state = LoopState::new();
    let mut pending = CapabilityResultEnvelope::ok(
        "media_download.download",
        Some("download".to_string()),
        json!({}),
    );
    pending.status = CapabilityResultStatus::Waiting;
    pending.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("job-model-synthesis".to_string()),
        poll_after_ms: Some(2_000),
        state: json!({}),
    });
    loop_state.capability_results.push(pending);
    let job = pending_async_job_ref_from_extra(Some(&json!({
        "pending_async_job": {
            "job_id": "job-model-synthesis",
            "status": "accepted",
            "poll_after_seconds": 2,
            "expires_at": 3000,
            "cancel_ref": "cancel:job-model-synthesis",
            "message_key": "clawd.task.async_job_pending"
        }
    })))
    .expect("parse")
    .expect("job");

    let payload = build_pending_async_job_checkpoint_progress_payload(
        &test_task(),
        &loop_state,
        "media_download",
        1,
        1,
        &job,
        None,
        1000,
        test_budget(&loop_state, 1),
    );
    let policy = &payload["task_checkpoint"]["boundary_context"]["async_completion_policy"];

    assert_eq!(policy["mode"], "continue_planning");
    assert_eq!(policy["continuation_action_count"], 0);
    assert_eq!(policy["delivery_intent"], "model_synthesis");
    assert_eq!(policy["requires_model_synthesis"], true);
    assert_eq!(
        policy["decision_reason_code"],
        "capability_result_requires_model_synthesis"
    );
}

#[test]
fn pending_exact_machine_job_keeps_direct_terminal_without_action_tail() {
    let mut loop_state = LoopState::new();
    let mut pending =
        CapabilityResultEnvelope::ok("machine.snapshot", Some("snapshot".to_string()), json!({}));
    pending.status = CapabilityResultStatus::Waiting;
    pending.delivery.intent = CapabilityDeliveryIntent::ExactMachine;
    pending.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("job-exact-machine".to_string()),
        poll_after_ms: Some(2_000),
        state: json!({}),
    });
    loop_state.capability_results.push(pending);
    let job = pending_async_job_ref_from_extra(Some(&json!({
        "pending_async_job": {
            "job_id": "job-exact-machine",
            "status": "accepted",
            "poll_after_seconds": 2,
            "expires_at": 3000,
            "cancel_ref": "cancel:job-exact-machine",
            "message_key": "clawd.task.async_job_pending"
        }
    })))
    .expect("parse")
    .expect("job");

    let payload = build_pending_async_job_checkpoint_progress_payload(
        &test_task(),
        &loop_state,
        "machine_snapshot",
        1,
        1,
        &job,
        None,
        1000,
        test_budget(&loop_state, 1),
    );
    let policy = &payload["task_checkpoint"]["boundary_context"]["async_completion_policy"];

    assert_eq!(policy["mode"], "direct_terminal");
    assert_eq!(policy["delivery_intent"], "exact_machine");
    assert_eq!(policy["requires_model_synthesis"], false);
    assert_eq!(policy["decision_reason_code"], "terminal_delivery_contract");
}

#[test]
fn pending_async_job_visible_reply_carries_checkpoint_markers() {
    let loop_state = LoopState::new();
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
    let loop_state = LoopState::new();
    let extra = json!({
        "execution_binding": {
            "skill_name": "video_generate",
            "version": "1.2.3",
            "manifest_digest": "manifest-digest",
            "receipt_digest": "receipt-digest",
            "registry_generation": 42,
            "registry_generation_digest": "generation-digest",
            "base_registry_digest": "base-digest",
            "overlay_generation_digest": "overlay-digest",
            "policy_digest": "policy-digest",
            "admission_receipt_digest": "admission-digest"
        },
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
        payload["task_checkpoint"]["boundary_context"]["async_poll_adapter"]["execution_binding"]
            ["registry_generation"],
        42
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["async_poll_adapter"]["execution_binding"]
            ["version"],
        "1.2.3"
    );
    assert_eq!(
        payload["task_lifecycle"]["async_timeout_policy"]["adapter_kind"],
        "skill_poll"
    );
    assert_eq!(
        payload["task_lifecycle"]["async_timeout_policy"]["policy_source"],
        "pending_async_job_contract"
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
