use serde_json::{json, Value};

use super::{
    apply_resume_steering_prompt, checkpoint_requires_stored_action,
    checkpoint_with_action_replay_binding, parse_checkpoint_continuation_actions,
};
use claw_core::skill_registry::SkillsRegistry;
use std::sync::{Arc, RwLock};

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
