use claw_core::model_turn::{ModelFinishReason, ModelToolCall, ModelTurnResponse};
use serde_json::json;

use super::*;

fn turn(tool_calls: Vec<ModelToolCall>, text: &str) -> ModelTurnResponse {
    ModelTurnResponse {
        text: text.to_string(),
        tool_calls,
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    }
}

fn repair_signal(error_code: &str, groups: &[&str]) -> String {
    json!({
        "protocol_observation": {
            "error_code": error_code,
            "tool_name": "load_capability_groups",
            "suggested_capability_groups": groups,
        }
    })
    .to_string()
}

#[test]
fn exact_group_loader_repair_completes_runtime_known_empty_arguments() {
    let signal = repair_signal("native_plan_unknown_tool", &["news_sources"]);

    for arguments in [
        json!({}),
        json!({"groups": []}),
        json!({"op": "load_groups"}),
    ] {
        let mut repaired = turn(
            vec![ModelToolCall {
                id: "loader-repair".to_string(),
                name: "load_capability_groups".to_string(),
                arguments,
            }],
            "",
        );

        assert_eq!(
            normalize_exact_capability_group_repair(&mut repaired, &signal),
            Some(vec!["news_sources".to_string()])
        );
        assert_eq!(
            repaired.tool_calls[0].arguments,
            json!({"op": "load_groups", "groups": ["news_sources"]})
        );
        assert!(matches!(
            super::super::action_from_native_capability_group_load(&repaired.tool_calls[0]),
            Ok(crate::AgentAction::CallTool { tool, args })
                if tool == "load_capability_groups"
                    && args == json!({"op": "load_groups", "groups": ["news_sources"]})
        ));
    }
}

#[test]
fn exact_group_loader_repair_does_not_guess_or_override_model_intent() {
    let exact_signal = repair_signal("native_plan_unknown_tool", &["news_sources"]);
    let no_candidate_signal = repair_signal("native_capability_group_load_groups_invalid", &[]);
    for (arguments, signal) in [
        (json!({}), no_candidate_signal.as_str()),
        (
            json!({"op": "search", "query": "financial news"}),
            exact_signal.as_str(),
        ),
        (
            json!({"groups": ["model_selected_group"]}),
            exact_signal.as_str(),
        ),
        (json!({"unexpected": true}), exact_signal.as_str()),
    ] {
        let original = arguments.clone();
        let mut repaired = turn(
            vec![ModelToolCall {
                id: "loader-no-repair".to_string(),
                name: "load_capability_groups".to_string(),
                arguments,
            }],
            "",
        );

        assert_eq!(
            normalize_exact_capability_group_repair(&mut repaired, signal),
            None
        );
        assert_eq!(repaired.tool_calls[0].arguments, original);
    }
}

#[test]
fn empty_group_loader_falls_back_to_local_catalog_search() {
    let mut model_query = turn(
        vec![ModelToolCall {
            id: "loader-search".to_string(),
            name: "load_capability_groups".to_string(),
            arguments: json!({}),
        }],
        "fetch financial news and provide summaries",
    );
    assert_eq!(
        normalize_empty_capability_loader_search(&mut model_query, "给我财经新闻"),
        Some("model_turn_text")
    );
    assert_eq!(
        model_query.tool_calls[0].arguments,
        json!({
            "op": "search",
            "query": "fetch financial news and provide summaries"
        })
    );

    let mut user_query = turn(
        vec![ModelToolCall {
            id: "loader-user-search".to_string(),
            name: "load_capability_groups".to_string(),
            arguments: json!({"groups": []}),
        }],
        "",
    );
    assert_eq!(
        normalize_empty_capability_loader_search(&mut user_query, "find a document parser"),
        Some("user_request")
    );
    assert_eq!(
        user_query.tool_calls[0].arguments,
        json!({"op": "search", "query": "find a document parser"})
    );
}

#[test]
fn empty_group_loader_search_preserves_explicit_or_ambiguous_calls() {
    for arguments in [
        json!({"op": "search", "query": "news"}),
        json!({"op": "expand", "capability_refs": ["capability:rss/latest"]}),
        json!({"groups": ["rss"]}),
        json!({"unexpected": true}),
    ] {
        let original = arguments.clone();
        let mut repaired = turn(
            vec![ModelToolCall {
                id: "loader-preserve".to_string(),
                name: "load_capability_groups".to_string(),
                arguments,
            }],
            "find news",
        );
        assert_eq!(
            normalize_empty_capability_loader_search(&mut repaired, "find news"),
            None
        );
        assert_eq!(repaired.tool_calls[0].arguments, original);
    }

    let mut multiple_calls = turn(
        vec![
            ModelToolCall {
                id: "loader-one".to_string(),
                name: "load_capability_groups".to_string(),
                arguments: json!({}),
            },
            ModelToolCall {
                id: "loader-two".to_string(),
                name: "load_capability_groups".to_string(),
                arguments: json!({}),
            },
        ],
        "find news",
    );
    assert_eq!(
        normalize_empty_capability_loader_search(&mut multiple_calls, "find news"),
        None
    );
}
