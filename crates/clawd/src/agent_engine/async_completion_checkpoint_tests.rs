use super::*;

fn poll_checkpoint() -> TaskCheckpoint {
    TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "agent-loop:task-1:round-1:step-1:async-job:provider:1".to_string(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "async_job_start_adapter",
            "agent_loop_resume_state": {
                "schema_version": 1,
                "stage": "tool_execution",
                "last_output": "pending",
                "history_compact": [],
                "task_observations": [],
                "executed_step_results": [{
                    "step_id": "step-1",
                    "skill": "video_generate",
                    "status": "ok",
                    "output": "pending",
                    "error": null,
                    "started_at": 10,
                    "finished_at": 11
                }]
            }
        }),
        last_successful_round: Some(1),
        last_successful_step: Some("step-1".to_string()),
        pending_action: None,
        observations: Vec::new(),
        capability_results: Vec::new(),
        evidence_refs: vec!["step-1".to_string()],
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec!["video_generate:hash".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 1,
            llm_calls: 1,
            tool_calls: 1,
            elapsed_ms: 20,
            llm_elapsed_ms: 10,
            tool_elapsed_ms: 10,
        },
        attempt_ledger: None,
        pending_async_job: Some(crate::task_lifecycle::AsyncJobRef {
            job_id: "provider:video:1".to_string(),
            status: crate::task_lifecycle::AsyncJobStatus::Running,
            poll_after_seconds: 2,
            expires_at: 1_000,
            runtime_deadline_at: None,
            retention_deadline_at: Some(1_000),
            cancel_ref: "provider:video:1".to_string(),
            message_key: "clawd.task.async_job_pending".to_string(),
        }),
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::PollAsyncJob,
    }
}

#[test]
fn completed_async_job_becomes_next_planner_round_with_terminal_evidence() {
    let mut pending_checkpoint = poll_checkpoint();
    let mut pending = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "media_download.transcribe",
        Some("transcribe".to_string()),
        json!({"output": "pending"}),
    );
    pending.status = claw_core::capability_result::CapabilityResultStatus::Waiting;
    pending.continuation = Some(claw_core::capability_result::Continuation {
        kind: claw_core::capability_result::ContinuationKind::Poll,
        reference: Some("provider:video:1".to_string()),
        poll_after_ms: Some(2_000),
        state: json!({}),
    });
    pending.provenance = json!({
        "task_id": "task-1",
        "action_fingerprint": "fp-audio-transcribe"
    });
    pending_checkpoint.capability_results.push(pending);
    let result = completed_async_job_continuation_result(
        "ask",
        "task-1",
        &pending_checkpoint,
        &json!({
            "status": "ok",
            "text": "saved",
            "extra": {
                "delivery": {"intent": "save_only", "deliver_to_user": false},
                "followup_policy": {
                    "capability": "audio.preview_transcribe",
                    "input_field": "audio_path",
                    "input_value": "/workspace/audio.wav"
                }
            }
        }),
        100,
    )
    .expect("continuation result");
    let checkpoint = crate::task_lifecycle::task_checkpoint_from_result_json(&result)
        .expect("successor checkpoint");

    assert_eq!(result["task_lifecycle"]["state"], "background");
    assert_eq!(result["task_lifecycle"]["next_check_after"], 100);
    assert!(checkpoint.checkpoint_id.ends_with(":completion"));
    assert_eq!(checkpoint.pending_async_job, None);
    assert_eq!(
        checkpoint.resume_entrypoint,
        ResumeEntrypoint::NextPlannerRound
    );
    assert_eq!(
        checkpoint.boundary_context["agent_loop_resume_state"]["last_output"],
        json!({
            "status": "ok",
            "text": "saved",
            "extra": {
                "delivery": {"intent": "save_only", "deliver_to_user": false},
                "followup_policy": {
                    "capability": "audio.preview_transcribe",
                    "input_field": "audio_path",
                    "input_value": "/workspace/audio.wav"
                }
            }
        })
        .to_string()
    );
    assert_eq!(
        checkpoint.boundary_context["agent_loop_resume_state"]["stage"],
        "planning"
    );
    assert_eq!(
        checkpoint.boundary_context["agent_loop_resume_state"]["async_completion_continuation"]
            ["continue_original_request"],
        true
    );
    assert!(
        checkpoint.boundary_context["agent_loop_resume_state"]["history_compact"]
            .as_array()
            .is_some_and(|history| history.iter().any(|entry| entry
                .as_str()
                .is_some_and(|entry| entry.contains("continue_original_request=true"))))
    );
    assert_eq!(
        checkpoint.boundary_context["agent_loop_resume_state"]["executed_step_results"][0]
            ["output"],
        checkpoint.boundary_context["agent_loop_resume_state"]["last_output"]
    );
    assert_eq!(checkpoint.observations.len(), 1);
    assert_eq!(
        checkpoint.boundary_context["async_capability_result_settled"],
        true
    );
    assert_eq!(
        checkpoint.capability_results[0].status,
        claw_core::capability_result::CapabilityResultStatus::Ok
    );
    assert_eq!(checkpoint.capability_results[0].continuation, None);
    assert_eq!(
        checkpoint.capability_results[0]
            .data
            .pointer("/extra/followup_policy/input_value"),
        Some(&json!("/workspace/audio.wav"))
    );
    let mut resumed_loop = crate::agent_engine::LoopState::new();
    crate::agent_engine::loop_state_seed::seed_loop_state_from_task_checkpoint(
        &mut resumed_loop,
        &checkpoint,
    );
    let planner_observation =
        crate::agent_engine::observed_output::latest_structured_capability_observation(
            &resumed_loop,
        )
        .expect("settled async result remains visible to the resumed planner");
    assert!(planner_observation.contains("audio.preview_transcribe"));
    assert!(planner_observation.contains("/workspace/audio.wav"));
    assert_eq!(
        checkpoint.completed_side_effect_refs,
        ["video_generate:hash"]
    );
}

#[test]
fn only_ask_agent_loop_poll_checkpoints_continue_planning() {
    let checkpoint = poll_checkpoint();
    assert!(completed_async_job_continuation_result(
        "run_skill",
        "task-1",
        &checkpoint,
        &json!({"status": "ok"}),
        100
    )
    .is_none());

    let mut non_agent = checkpoint.clone();
    non_agent.checkpoint_id = "skill-job:1".to_string();
    assert!(completed_async_job_continuation_result(
        "ask",
        "task-1",
        &non_agent,
        &json!({"status": "ok"}),
        100
    )
    .is_none());
}

#[test]
fn completed_single_action_async_job_projects_terminal_without_provider_round() {
    let mut checkpoint = poll_checkpoint();
    checkpoint.boundary_context["async_completion_policy"] = json!({
        "schema_version": 1,
        "mode": "direct_terminal",
        "continuation_action_count": 0,
        "continuation_actions": [],
    });

    assert!(completed_async_job_continuation_result(
        "ask",
        "task-1",
        &checkpoint,
        &json!({"status": "ok", "output": "done", "exit_code": 0}),
        100
    )
    .is_none());
}

#[test]
fn failed_async_stt_resumes_planner_with_exact_local_fallback_input() {
    let mut checkpoint = poll_checkpoint();
    let mut pending = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "audio.transcribe",
        Some("transcribe".to_string()),
        json!({"output": "pending"}),
    );
    pending.status = claw_core::capability_result::CapabilityResultStatus::Waiting;
    pending.continuation = Some(claw_core::capability_result::Continuation {
        kind: claw_core::capability_result::ContinuationKind::Poll,
        reference: Some("provider:video:1".to_string()),
        poll_after_ms: Some(2_000),
        state: json!({}),
    });
    pending.provenance = json!({
        "task_id": "task-1",
        "action_fingerprint": "fp-audio-transcribe"
    });
    checkpoint.capability_results.push(pending);

    let result = failed_async_job_continuation_result(
        "ask",
        "task-1",
        &checkpoint,
        &json!({
            "schema_version": 1,
            "status": "error",
            "extra": {
                "schema_version": 1,
                "source_skill": "audio_transcribe",
                "status": "error",
                "error_code": "provider_request_failed",
                "message_key": "skill.audio_transcribe.provider_request_failed",
                "retryable": true,
                "fallback_capability": "media_download.transcribe",
                "fallback_input_field": "input_path",
                "fallback_input_value": "/workspace/extracted.wav"
            }
        }),
        100,
    )
    .expect("failed async capability remains recoverable");
    let resumed = crate::task_lifecycle::task_checkpoint_from_result_json(&result)
        .expect("successor checkpoint");

    assert_eq!(result["task_lifecycle"]["state"], "background");
    assert_eq!(
        result["task_lifecycle"]["resume_reason"],
        "async_job_failed_continue_recovery"
    );
    assert_eq!(
        resumed.capability_results[0].status,
        claw_core::capability_result::CapabilityResultStatus::Error
    );
    assert_eq!(
        resumed.capability_results[0]
            .error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("provider_request_failed")
    );
    assert_eq!(
        resumed.capability_results[0]
            .data
            .pointer("/extra/fallback_input_value"),
        Some(&json!("/workspace/extracted.wav"))
    );
    assert_eq!(
        resumed.capability_results[0].provenance["task_id"],
        "task-1"
    );
    assert_eq!(
        resumed.capability_results[0].provenance["action_fingerprint"],
        "fp-audio-transcribe"
    );
}
