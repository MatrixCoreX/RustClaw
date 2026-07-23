use serde_json::json;

use super::*;

fn checkpoint_for_stage(stage: AgentCheckpointStage) -> crate::task_lifecycle::TaskCheckpoint {
    let mut source = LoopState::new();
    source.last_output = Some("machine-observation".to_string());
    source.history_compact = vec!["step=1 status=ok".to_string()];
    source
        .task_observations
        .push(json!({"kind": "tool_result", "status_code": "ok"}));
    source.latest_validation_result = Some(json!({
        "status_code": "verified",
        "changed_files": ["src/lib.rs"]
    }));
    source.delivery_messages = vec!["completed-result".to_string()];
    source.last_user_visible_respond = Some("completed-result".to_string());
    source.last_publishable_synthesis_output = Some("synthesized-result".to_string());
    source.last_capability_synthesis_output = Some("capability-result".to_string());
    source.loaded_capability_skills.insert("crypto".to_string());

    crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: format!("checkpoint-{}", stage.as_str()),
        boundary_context: json!({
            "schema_version": 1,
            "source": "stage_restart_matrix",
            "agent_loop_resume_state": build_checkpoint_resume_state(&source, stage),
        }),
        last_successful_round: Some(2),
        last_successful_step: Some("step_1".to_string()),
        pending_action: None,
        observations: vec![json!({"step_id": "step_1", "status": "ok"})],
        capability_results: Vec::new(),
        evidence_refs: vec!["step_1".to_string()],
        artifact_refs: vec!["changed_file:src/lib.rs".to_string()],
        completed_side_effect_refs: vec!["mutation:fingerprint".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 2,
            step: 3,
            llm_calls: 2,
            tool_calls: 1,
            elapsed_ms: 400,
            llm_elapsed_ms: 250,
            tool_elapsed_ms: 150,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    }
}

#[test]
fn restart_matrix_restores_all_agent_phase_machine_state() {
    for stage in [
        AgentCheckpointStage::Planning,
        AgentCheckpointStage::ToolExecution,
        AgentCheckpointStage::Verification,
        AgentCheckpointStage::PatchReview,
        AgentCheckpointStage::FinalSynthesis,
    ] {
        let checkpoint = checkpoint_for_stage(stage);
        let mut restored = LoopState::new();

        crate::agent_engine::seed_loop_state_from_task_checkpoint(&mut restored, &checkpoint);

        assert_eq!(
            restored
                .output_vars
                .get("agent_loop.resume_stage")
                .map(String::as_str),
            Some(stage.as_str())
        );
        assert_eq!(restored.last_output.as_deref(), Some("machine-observation"));
        assert!(restored
            .history_compact
            .iter()
            .any(|entry| entry == "step=1 status=ok"));
        assert!(restored.task_observations.iter().any(|item| {
            item.get("status_code").and_then(serde_json::Value::as_str) == Some("ok")
        }));
        assert_eq!(
            restored
                .latest_validation_result
                .as_ref()
                .and_then(|value| value.get("status_code"))
                .and_then(serde_json::Value::as_str),
            Some("verified")
        );
        assert_eq!(
            restored.last_publishable_synthesis_output.as_deref(),
            Some("synthesized-result")
        );
        assert_eq!(
            restored.last_capability_synthesis_output.as_deref(),
            Some("capability-result")
        );
        assert_eq!(
            restored.loaded_capability_skills,
            std::collections::BTreeSet::from(["crypto".to_string()])
        );
        assert_eq!(
            restored
                .successful_action_fingerprints
                .get("mutation:fingerprint"),
            Some(&1)
        );
    }
}

#[test]
fn restart_snapshot_is_bounded_and_ignores_unknown_stage_tokens() {
    let mut source = LoopState::new();
    source.last_output = Some("x".repeat(MAX_LAST_OUTPUT_CHARS + 100));
    source
        .task_observations
        .push(json!({"payload": "x".repeat(MAX_OBSERVATION_BYTES + 100)}));
    let mut snapshot = build_checkpoint_resume_state(&source, AgentCheckpointStage::ToolExecution);
    snapshot["stage"] = json!("untrusted_stage");
    snapshot["loaded_capability_skills"] = json!([
        "crypto",
        "weather",
        "bad group",
        "rss_fetch",
        "kb",
        "extra_group"
    ]);
    let boundary = json!({"agent_loop_resume_state": snapshot});
    let mut restored = LoopState::new();

    let stage = restore_checkpoint_resume_state(&mut restored, &boundary);

    assert_eq!(stage, AgentCheckpointStage::Planning);
    assert_eq!(
        restored.last_output.as_deref().map(str::len),
        Some(MAX_LAST_OUTPUT_CHARS)
    );
    assert!(restored.task_observations.is_empty());
    assert_eq!(
        restored.loaded_capability_skills,
        std::collections::BTreeSet::from([
            "crypto".to_string(),
            "kb".to_string(),
            "rss_fetch".to_string(),
            "weather".to_string(),
        ])
    );
}
