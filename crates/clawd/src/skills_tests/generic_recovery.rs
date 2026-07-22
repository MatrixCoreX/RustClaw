use super::super::{
    is_recoverable_skill_error, skill_error_machine_observation, structured_skill_error_string,
};
use serde_json::json;

#[test]
fn structured_pre_dispatch_argument_failure_allows_generic_bounded_replan() {
    let encoded = structured_skill_error_string(
        "custom_observer",
        &json!({
            "status": "error",
            "error_text": "configured selector was not found",
            "extra": {
                "error_kind": "selector_not_configured",
                "retryable": true,
                "failure_phase": "pre_dispatch",
                "side_effect_applied": false,
                "recovery_action": "replan_arguments",
                "invalid_argument": "selector",
                "available_values": ["default"]
            }
        }),
    );

    assert!(is_recoverable_skill_error("custom_observer", &encoded));
    let observation: serde_json::Value = serde_json::from_str(
        &skill_error_machine_observation("custom_observer", &encoded).unwrap(),
    )
    .unwrap();
    assert_eq!(
        observation
            .pointer("/extra/recovery_action")
            .and_then(serde_json::Value::as_str),
        Some("replan_arguments")
    );
    assert!(observation.pointer("/error_text").is_none());
}

#[test]
fn generic_bounded_replan_contract_fails_closed_after_possible_side_effect() {
    let encoded = structured_skill_error_string(
        "custom_mutator",
        &json!({
            "status": "error",
            "error_text": "mutation status is uncertain",
            "extra": {
                "error_kind": "argument_rejected",
                "retryable": true,
                "failure_phase": "pre_dispatch",
                "side_effect_applied": true,
                "recovery_action": "replan_arguments"
            }
        }),
    );

    assert!(!is_recoverable_skill_error("custom_mutator", &encoded));
}
