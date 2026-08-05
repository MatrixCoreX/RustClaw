use std::collections::BTreeMap;

use claw_core::model_turn::{
    ModelMessage, ModelRole, ModelToolChoice, ModelToolDefinition, ModelTurnRequest,
};
use serde_json::json;

use super::*;

#[test]
fn preserves_authoritative_context_and_tool_catalog() {
    let policy = crate::task_context_builder::ContextWindowPolicy::new(
        "fixture-provider".to_string(),
        "fixture-model".to_string(),
        4_096,
        512,
        512,
        256,
        1_000,
        crate::task_context_builder::ContextTokenScope::Total,
    );
    let system = ModelMessage::text(ModelRole::System, "authoritative-system-instruction");
    let request = ModelTurnRequest {
        messages: vec![
            system.clone(),
            ModelMessage::text(ModelRole::User, "dynamic-history ".repeat(4_000)),
        ],
        tools: vec![ModelToolDefinition {
            name: "fixture.lookup".to_string(),
            description: "fixture tool".to_string(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"}}}),
            strict: true,
        }],
        tool_choice: ModelToolChoice::Required,
        response_schema: Some(json!({"type":"object"})),
        stream: false,
        metadata: BTreeMap::new(),
    };

    let (prompt, recovered, observation) =
        build_context_length_recovery_request(&policy, &request, "original prompt");

    assert_eq!(recovered.messages[0], system);
    assert_eq!(recovered.tools, request.tools);
    assert_eq!(recovered.tool_choice, request.tool_choice);
    assert_eq!(recovered.response_schema, request.response_schema);
    assert!(
        serde_json::to_string(&recovered).unwrap().len()
            < serde_json::to_string(&request).unwrap().len()
    );
    assert!(prompt.contains("context_recovery_gap"));
    assert_eq!(observation["retry_limit"], 1);
    assert_eq!(observation["system_context_preserved"], true);
    assert_eq!(observation["tool_catalog_preserved"], true);
    assert_eq!(observation["completed_side_effect_replay"], false);
    assert!(observation.get("original_prompt").is_none());
}

#[test]
fn compactor_is_noop_below_budget_and_data_only_above_it() {
    assert_eq!(compact_text_to_token_budget("short", 100), "short");
    let compacted = compact_text_to_token_budget(&"abcdef".repeat(2_000), 256);
    assert!(compacted.contains("data_only=\"true\""));
    assert!(compacted.contains("canonical_events_preserved=\"true\""));
    assert!(compacted.len() < 12_000);
}
