use serde_json::json;

use super::{plan_actions_follow_machine_contract, plan_json_has_unterminated_string};
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
