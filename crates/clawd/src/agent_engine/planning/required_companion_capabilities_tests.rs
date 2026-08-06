use claw_core::capability_result::{CapabilityResultEnvelope, RetryDirective, StructuredError};
use claw_core::model_turn::{ModelFinishReason, ModelToolCall, ModelTurnResponse};
use serde_json::json;

use super::super::{
    actions_from_native_turn_with_groups, native_capability_leaf_tool_name,
    native_contract_repair_signal_for_turn, native_contract_retry_request, native_planner_request,
    normalize_exact_capability_group_repair,
};
use super::*;

fn turn(tool_calls: Vec<ModelToolCall>) -> ModelTurnResponse {
    ModelTurnResponse {
        text: String::new(),
        tool_calls,
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    }
}

fn respond_call(content: &str) -> ModelToolCall {
    ModelToolCall {
        id: "companion-respond".to_string(),
        name: "respond".to_string(),
        arguments: json!({
            "shape": "free_text",
            "content": content,
            "items": [],
            "exact_item_count": 0,
            "fields": [],
            "observed_fields": [],
            "exact_field_count": 0
        }),
    }
}

fn resolved_observation(primary: &str, companions: &[&str]) -> serde_json::Value {
    json!({
        "observation_kind": "capability_resolution",
        "outcome": "resolved",
        "requested_capability": primary,
        "required_companions": companions,
    })
}

#[test]
fn companions_activate_only_after_primary_success() {
    let mut state = LoopState::default();
    state.task_observations.push(resolved_observation(
        "web.search_results",
        &["rss.latest_news"],
    ));
    assert!(missing_required_companion_capabilities(&state).is_empty());

    state.capability_results.push(CapabilityResultEnvelope::ok(
        "web.search_results",
        None,
        json!({"items": []}),
    ));
    assert_eq!(
        missing_required_companion_capabilities(&state),
        vec!["rss.latest_news"]
    );
}

#[test]
fn successful_and_terminal_unavailable_companions_settle_obligation() {
    let mut state = LoopState::default();
    state.task_observations.push(resolved_observation(
        "web.search_results",
        &["rss.list_categories", "rss.latest_news"],
    ));
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "web.search_results",
        None,
        json!({}),
    ));
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "rss.list_categories",
        None,
        json!({}),
    ));
    state
        .capability_results
        .push(CapabilityResultEnvelope::failed(
            "rss.latest_news",
            None,
            StructuredError {
                code: "capability_unavailable".to_string(),
                message_key: "capability.unavailable".to_string(),
                retryable: false,
                details: json!({}),
            },
        ));

    assert!(missing_required_companion_capabilities(&state).is_empty());
}

#[test]
fn retryable_companion_failure_remains_missing() {
    let mut state = LoopState::default();
    state.task_observations.push(resolved_observation(
        "web.search_results",
        &["rss.latest_news"],
    ));
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "web.search_results",
        None,
        json!({}),
    ));
    let mut failed = CapabilityResultEnvelope::failed(
        "rss.latest_news",
        None,
        StructuredError {
            code: "upstream_timeout".to_string(),
            message_key: "rss.upstream_timeout".to_string(),
            retryable: true,
            details: json!({}),
        },
    );
    failed.retry = Some(RetryDirective {
        retryable: true,
        class: None,
        after_ms: None,
    });
    state.capability_results.push(failed);

    assert_eq!(
        missing_required_companion_capabilities(&state),
        vec!["rss.latest_news"]
    );
}

#[test]
fn native_respond_waits_for_required_companion_capabilities() {
    let mut state = LoopState::default();
    state.task_observations.push(resolved_observation(
        "web.search_results",
        &["rss.list_categories", "rss.latest_news"],
    ));
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "web.search_results",
        None,
        json!({"items": []}),
    ));
    let response = turn(vec![respond_call("Done.")]);

    assert_eq!(
        actions_from_native_turn_with_groups(&response, &[], &Default::default(), Some(&state))
            .expect_err("final response must wait for companion observations"),
        "native_plan_required_companion_capability_missing"
    );

    for capability in ["rss.list_categories", "rss.latest_news"] {
        state
            .capability_results
            .push(CapabilityResultEnvelope::ok(capability, None, json!({})));
    }
    assert!(actions_from_native_turn_with_groups(
        &response,
        &[],
        &Default::default(),
        Some(&state),
    )
    .is_ok());
}

#[test]
fn missing_companion_repair_loads_the_exact_registry_group() {
    let group = crate::capability_map::PlannerNativeCapabilityGroup {
        skill_name: "rss".to_string(),
        tool_name: "call_rss".to_string(),
        description: "runtime_capability_group_v1; semantic_tags=news".to_string(),
        capability_names: vec![
            "rss.list_categories".to_string(),
            "rss.latest_news".to_string(),
        ],
        capability_descriptions: Default::default(),
        capability_argument_schemas: Default::default(),
    };
    let request = native_planner_request(
        "protocol",
        "current turn",
        None,
        &[],
        &Default::default(),
        std::slice::from_ref(&group),
        &[],
        &["rss".to_string()],
    );
    let malformed = turn(vec![respond_call("premature")]);
    let mut state = LoopState::default();
    state.task_observations.push(resolved_observation(
        "web.search_results",
        &["rss.list_categories", "rss.latest_news"],
    ));
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "web.search_results",
        None,
        json!({}),
    ));

    let signal = native_contract_repair_signal_for_turn(
        "native_plan_required_companion_capability_missing",
        &malformed,
        &request,
        std::slice::from_ref(&group),
        &["rss".to_string()],
        &Default::default(),
        &Default::default(),
        Some(&state),
        &[],
    );
    let observation: serde_json::Value = serde_json::from_str(&signal).expect("repair observation");
    assert_eq!(
        observation["protocol_observation"]["tool_name"],
        "load_capability_groups"
    );
    assert_eq!(
        observation["protocol_observation"]["suggested_capability_groups"],
        json!(["rss"])
    );
    assert_eq!(
        observation["protocol_observation"]["required_companion_capabilities"],
        json!(["rss.latest_news", "rss.list_categories"])
    );
    let repaired_request = native_contract_retry_request(&request, &signal);
    assert_eq!(repaired_request.tools.len(), 1);
    assert_eq!(repaired_request.tools[0].name, "load_capability_groups");

    let mut empty_repair = turn(vec![ModelToolCall {
        id: "companion-loader".to_string(),
        name: "load_capability_groups".to_string(),
        arguments: json!({}),
    }]);
    assert_eq!(
        normalize_exact_capability_group_repair(&mut empty_repair, &signal),
        Some(vec!["rss".to_string()])
    );
    assert_eq!(
        empty_repair.tool_calls[0].arguments,
        json!({"op": "load_groups", "groups": ["rss"]})
    );
}

#[test]
fn missing_loaded_companion_repair_calls_disclosed_leaf_without_reloading() {
    let missing = "rss.latest_news";
    let group = crate::capability_map::PlannerNativeCapabilityGroup {
        skill_name: "rss".to_string(),
        tool_name: "call_rss".to_string(),
        description: "runtime_capability_group_v1; semantic_tags=news".to_string(),
        capability_names: vec!["rss.list_categories".to_string(), missing.to_string()],
        capability_descriptions: Default::default(),
        capability_argument_schemas: Default::default(),
    };
    let request = native_planner_request(
        "protocol",
        "current turn",
        None,
        &[],
        &Default::default(),
        std::slice::from_ref(&group),
        std::slice::from_ref(&group),
        &[],
    );
    let malformed = turn(vec![respond_call("premature")]);
    let mut state = LoopState::default();
    state.task_observations.push(resolved_observation(
        "web.search_results",
        &["rss.list_categories", missing],
    ));
    for capability in ["web.search_results", "rss.list_categories"] {
        state
            .capability_results
            .push(CapabilityResultEnvelope::ok(capability, None, json!({})));
    }

    let signal = native_contract_repair_signal_for_turn(
        "native_plan_required_companion_capability_missing",
        &malformed,
        &request,
        std::slice::from_ref(&group),
        &[],
        &Default::default(),
        &Default::default(),
        Some(&state),
        &[],
    );
    let observation: serde_json::Value = serde_json::from_str(&signal).expect("repair observation");
    let leaf = native_capability_leaf_tool_name(missing);
    assert!(observation["protocol_observation"]["tool_name"].is_null());
    assert_eq!(
        observation["protocol_observation"]["suggested_capability_groups"],
        json!([])
    );
    assert_eq!(
        observation["protocol_observation"]["available_tool_names"],
        json!([leaf])
    );
    let repaired_request = native_contract_retry_request(&request, &signal);
    assert_eq!(repaired_request.tools.len(), 1);
    assert_eq!(repaired_request.tools[0].name, leaf);
}
