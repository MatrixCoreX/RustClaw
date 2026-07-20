use serde_json::{json, Value};

use super::{
    apply_resume_steering_prompt, checkpoint_requires_stored_action,
    parse_checkpoint_continuation_actions,
};

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
    assert_eq!(envelope["protocol"], "rustclaw.resume_input.v1");
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
fn only_confirmation_checkpoint_requires_private_stored_action() {
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

    checkpoint.pending_action = None;
    assert!(!checkpoint_requires_stored_action(&checkpoint));
}
