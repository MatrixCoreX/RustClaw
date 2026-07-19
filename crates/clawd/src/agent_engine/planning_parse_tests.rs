use serde_json::json;

use super::{
    parse_plan_action_step, parse_single_plan_actions, plan_actions_follow_machine_contract,
    plan_json_has_unterminated_string,
};
use crate::AgentAction;

fn state_with_workspace_registry() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state
}

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "planning-parse-test".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn unterminated_terminal_response_string_is_rejected() {
    let raw = r#"{"steps":[{"type":"respond","content":"complete prefix then trunc"#;

    assert!(plan_json_has_unterminated_string(raw));
}

#[test]
fn complete_plan_with_escaped_quotes_is_not_rejected() {
    let raw = r#"{"steps":[{"type":"respond","content":"a \"quoted\" complete answer"}]}"#;

    assert!(!plan_json_has_unterminated_string(raw));
}

#[test]
fn non_json_tool_protocol_is_left_for_its_own_parser() {
    let raw = r#"<invoke name="call_tool"><parameter name="tool">demo</parameter></invoke>"#;

    assert!(!plan_json_has_unterminated_string(raw));
}

#[test]
fn planner_machine_contract_rejects_unknown_capability() {
    let state = state_with_workspace_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.remove_tree".to_string(),
        args: json!({"path": "tmp/example"}),
    }];

    assert!(!plan_actions_follow_machine_contract(&state, &actions));
}

#[test]
fn planner_machine_contract_accepts_resolvable_capability() {
    let state = state_with_workspace_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.remove_path".to_string(),
        args: json!({"path": "tmp/example"}),
    }];

    assert!(plan_actions_follow_machine_contract(&state, &actions));
}

#[tokio::test]
async fn invalid_optional_output_contract_does_not_discard_valid_steps() {
    let state = state_with_workspace_registry();
    let raw = r#"{
      "output_contract": {
        "response_shape": "strict",
        "exact_sentence_count": null,
        "requires_content_evidence": true,
        "delivery_required": false,
        "locator_kind": "none",
        "delivery_intent": "none",
        "result_kind": "invented_permission_preview"
      },
      "steps": [
        {
          "type": "call_capability",
          "capability": "system.preview_command_permission",
          "args": {"command": "sudo rm -rf /tmp/rustclaw-never-run"}
        },
        {"type": "synthesize_answer", "evidence_refs": ["last_output"]}
      ]
    }"#;
    let schema_error = crate::prompt_utils::validate_against_schema::<serde_json::Value>(
        raw,
        crate::prompt_utils::PromptSchemaId::PlanResult,
    )
    .expect_err("fixture must contain only an invalid output contract");
    assert!(
        schema_error.contract_violations_only_under("$.output_contract"),
        "{schema_error}"
    );
    let raw_value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
    let first_step = raw_value.pointer("/steps/0").expect("first fixture step");
    let first_action = parse_plan_action_step(first_step, &state).expect("first action must parse");
    assert!(
        plan_actions_follow_machine_contract(&state, &[first_action]),
        "first action must satisfy the registry contract"
    );

    let actions = parse_single_plan_actions(raw, &state, &task())
        .await
        .expect("valid steps should survive an invalid optional output contract");

    assert!(matches!(
        actions.first(),
        Some(AgentAction::CallCapability { capability, .. })
            if capability == "system.preview_command_permission"
    ));
    assert!(matches!(
        actions.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[tokio::test]
async fn output_contract_recovery_does_not_bypass_capability_resolution() {
    let state = state_with_workspace_registry();
    let raw = r#"{
      "output_contract": {
        "response_shape": "strict",
        "exact_sentence_count": null,
        "requires_content_evidence": true,
        "delivery_required": false,
        "locator_kind": "none",
        "delivery_intent": "none",
        "result_kind": "invented_permission_preview",
        "structured_field_selector": "decision"
      },
      "steps": [
        {
          "type": "call_capability",
          "capability": "system.invented_permission_preview",
          "args": {"command": "true"}
        }
      ]
    }"#;

    assert!(
        parse_single_plan_actions(raw, &state, &task())
            .await
            .is_none(),
        "discarding an optional output contract must not bypass capability resolution"
    );
}
