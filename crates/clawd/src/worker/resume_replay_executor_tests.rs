use serde_json::{json, Value};

use super::{
    apply_resume_steering_prompt, checkpoint_requires_stored_action,
    checkpoint_with_action_replay_binding, parse_checkpoint_continuation_actions,
};
use claw_core::skill_registry::SkillsRegistry;
use std::sync::{Arc, RwLock};

#[tokio::test]
async fn historical_quota_dispatch_terminates_before_input_materialization_and_model_preparation() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.core.llm_providers.clear();
    state.seed_ask_task_row("quota-resume-dispatch", 7, 11, r#"{"text":"继续执行"}"#);
    let task = crate::repo::claim_next_task(&state).unwrap().unwrap();
    let checkpoint_id = "quota-checkpoint";
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.into(),
        boundary_context: json!({"resume_reason":"provider_blocker_wait_background",
            "provider_status":{"provider":"fixture","status_code":"quota_exhausted","external_provider_blocked":true},
            "task_llm_metrics":{"schema_version":1,"llm_calls":2,"llm_elapsed_ms":1,"by_prompt":{},"call_sequence":[]}}),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: None,
        observations: vec![],
        capability_results: vec![],
        evidence_refs: vec![],
        artifact_refs: vec![],
        completed_side_effect_refs: vec![],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 0,
            llm_calls: 2,
            tool_calls: 0,
            elapsed_ms: 1,
            llm_elapsed_ms: 1,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let claimed = crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
        task: task.clone(),
        task_id: task.task_id.clone(),
        checkpoint_id: checkpoint_id.into(),
        executor_state: "executing_planner_resume".into(),
        executor_action: "run_seeded_agent_loop".into(),
        executor_status: "seeded_loop_requires_provider_window".into(),
        dispatch_state: "ready_to_run_seeded_agent_loop".into(),
        dispatch_execution_state: "claimed_to_run_seeded_agent_loop".into(),
        resume_trigger: "worker_recovery".into(),
        resume_directive: "run_next_planner_round".into(),
        lease_expires_at: 100,
        handoff_claim_expires_at: 90,
        dispatch_claim_expires_at: 80,
        execution_plan: json!({}),
        dispatch_payload: json!({}),
        dispatch_claim: json!({}),
        task_checkpoint: checkpoint,
    };
    let result = super::execute_seeded_agent_loop_dispatch_result(&state, &claimed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["executor_result_status"], "seeded_loop_failed");
    let failure = &result["failure_result_json"];
    assert_eq!(failure["text"], "API quota exhausted. Task terminated.");
    assert_eq!(
        failure["task_journal"]["summary"]["task_metrics"]["llm_calls_per_task"],
        2
    );
    assert!(failure["task_journal"]["summary"]["task_checkpoint"].is_null());
    assert_eq!(state.task_llm_call_count(&task.task_id), 2);
}

fn resource_replay_test_state() -> crate::AppState {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-resource-replay-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&root).expect("create replay fixture");
    let registry_path = root.join("skills_registry.toml");
    std::fs::write(
        &registry_path,
        r#"
[[skills]]
name = "browser_session"
enabled = true
kind = "builtin"
planner_kind = "tool"
planner_capabilities = [
  { name = "browser.session_open", action = "session_open", effect = "observe" },
]
"#,
    )
    .expect("write registry fixture");
    let registry = Arc::new(SkillsRegistry::load_from_path(&registry_path).expect("load registry"));
    let _ = std::fs::remove_dir_all(root);
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(registry),
        skills_list: Arc::new(["browser_session".to_string()].into_iter().collect()),
    })));
    state
}

#[test]
fn checkpoint_continuation_parser_accepts_actions_and_rejects_malformed_suffix() {
    let actions = parse_checkpoint_continuation_actions(Some(json!([
        {
            "type": "call_capability",
            "capability": "filesystem.write_text",
            "args": {"path": "run/result.txt", "content": "ok"}
        },
        {"type": "synthesize_answer", "evidence_refs": ["s2"]}
    ])))
    .expect("valid continuation");
    assert_eq!(actions.len(), 2);

    let error = parse_checkpoint_continuation_actions(Some(json!([
        {"type": "call_capability", "args": {}}
    ])))
    .expect_err("malformed continuation must fail closed");
    assert_eq!(error.to_string(), "checkpoint_continuation_actions_invalid");
}

#[test]
fn resume_steering_prompt_preserves_multilingual_input_as_opaque_json() {
    let mut payload = json!({"text": "initial request"});
    let input = json!({
        "user_message": "继续，但不要改公开接口",
        "new_constraints": {
            "verification": "必須",
            "scope": ["src"]
        }
    });

    apply_resume_steering_prompt(&mut payload, &input);

    let envelope: Value =
        serde_json::from_str(payload["text"].as_str().expect("steering prompt")).expect("JSON");
    assert_eq!(envelope["protocol"], "agent.resume_input.v1");
    assert_eq!(envelope["original_request"], "initial request");
    assert_eq!(envelope["user_message"], "继续，但不要改公开接口");
    assert_eq!(envelope["new_constraints"]["verification"], "必須");
    assert_eq!(envelope["new_constraints"]["scope"], json!(["src"]));
}

#[test]
fn resume_steering_prompt_supports_constraint_only_resume() {
    let mut payload = json!({"text": "initial request"});

    apply_resume_steering_prompt(
        &mut payload,
        &json!({"new_constraints": {"budget_profile": "long_tail"}}),
    );

    let envelope: Value =
        serde_json::from_str(payload["text"].as_str().expect("steering prompt")).expect("JSON");
    assert!(envelope.get("user_message").is_none());
    assert_eq!(envelope["new_constraints"]["budget_profile"], "long_tail");
}

#[test]
fn replayable_checkpoint_kinds_require_private_stored_action() {
    let mut checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "checkpoint-1".to_string(),
        boundary_context: json!({}),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: Some(json!({
            "kind": "agent_hook_pre_tool_use",
            "action_ref": "system.run_command",
            "args_keys": ["command"]
        })),
        observations: Vec::new(),
        capability_results: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 1,
            llm_calls: 1,
            tool_calls: 0,
            elapsed_ms: 1,
            llm_elapsed_ms: 1,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    assert!(checkpoint_requires_stored_action(&checkpoint));

    checkpoint.pending_action = Some(json!({
        "kind": "resource_admission_retry",
        "action_ref": "browser.session_open",
        "args_keys": ["action", "url"]
    }));
    assert!(checkpoint_requires_stored_action(&checkpoint));

    checkpoint.pending_action = None;
    assert!(!checkpoint_requires_stored_action(&checkpoint));
}

#[test]
fn resource_replay_rebinds_capability_to_exact_runtime_action() {
    let state = resource_replay_test_state();
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "checkpoint-resource".to_string(),
        boundary_context: json!({}),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: Some(json!({
            "kind": "resource_admission_retry",
            "action_ref": "browser.session_open",
            "args_keys": ["url"]
        })),
        observations: Vec::new(),
        capability_results: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 2,
            step: 2,
            llm_calls: 2,
            tool_calls: 1,
            elapsed_ms: 1,
            llm_elapsed_ms: 1,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let action = crate::repo::TaskCheckpointAction {
        task_id: "task-resource".to_string(),
        checkpoint_id: "checkpoint-resource".to_string(),
        tool_or_skill: "browser_session".to_string(),
        action_ref: "browser.session_open".to_string(),
        args: json!({"url": "https://example.invalid"}),
        output_contract: None,
        continuation_actions: None,
        execution_binding: Some(json!({"schema_version": 1})),
        approval_binding: None,
        instruction_revision: 0,
        execution_epoch: 0,
    };

    let replay = checkpoint_with_action_replay_binding(&state, &checkpoint, &action)
        .expect("bind replay action");

    assert_eq!(
        replay.boundary_context["checkpoint_action_replay"]["action_ref"],
        "browser_session.session_open"
    );
    assert_eq!(
        replay.boundary_context["checkpoint_action_replay"]["capability_ref"],
        "browser.session_open"
    );
}
