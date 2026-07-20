use claw_core::capability_result::{
    CapabilityDelivery, CapabilityDeliveryIntent, CapabilityResultEnvelope,
};
use serde_json::json;

use super::{bounded_result, eligible_for_capability_result_synthesis, MAX_RESULT_JSON_CHARS};
use crate::agent_engine::{AgentRunContext, LoopState};

#[test]
fn ordinary_free_response_uses_generic_synthesis() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "filesystem.list",
            Some("list".to_string()),
            json!({"entries": ["README.md"]}),
        ));
    assert!(eligible_for_capability_result_synthesis(
        &loop_state,
        Some(&AgentRunContext::default())
    ));
}

#[test]
fn exact_machine_and_artifact_delivery_bypass_language_synthesis() {
    let mut loop_state = LoopState::default();
    let mut result =
        CapabilityResultEnvelope::ok("filesystem.read", Some("read".to_string()), json!({}));
    result.delivery = CapabilityDelivery {
        intent: CapabilityDeliveryIntent::ExactMachine,
        constraints: json!({}),
    };
    loop_state.capability_results.push(result);
    assert!(!eligible_for_capability_result_synthesis(&loop_state, None));
}

#[test]
fn oversized_result_is_bounded_without_changing_machine_identity() {
    let result = CapabilityResultEnvelope::ok(
        "filesystem.read",
        Some("read".to_string()),
        json!({"content": "x".repeat(MAX_RESULT_JSON_CHARS + 10_000)}),
    );
    let bounded = bounded_result(&result);
    assert_eq!(bounded.capability, result.capability);
    assert_eq!(bounded.action, result.action);
    assert!(bounded.data.to_string().chars().count() < MAX_RESULT_JSON_CHARS);
}
