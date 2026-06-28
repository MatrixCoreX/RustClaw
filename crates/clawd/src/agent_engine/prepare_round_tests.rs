use super::{
    verifier_gate_missing_slots, verifier_gate_needs_clarification, verifier_gate_should_stop_round,
};

fn verify_result_with_issue(
    mode: crate::verifier::VerifyMode,
    kind: crate::verifier::VerifyIssueKind,
) -> crate::verifier::VerifyResult {
    crate::verifier::VerifyResult {
        mode,
        approved: matches!(mode, crate::verifier::VerifyMode::ObserveOnly),
        blocked_reason: None,
        shadow_blocked_reason: Some(kind.as_str().to_string()),
        permission_decision: serde_json::json!({
            "schema_version": 1,
            "owner_layer": "plan_verifier",
        }),
        approved_steps: Vec::new(),
        needs_confirmation: false,
        rewritten_steps: Vec::new(),
        issues: vec![crate::verifier::VerifyIssue {
            step_id: "step_1".to_string(),
            kind,
            detail: format!("test issue {}", kind.as_str()),
            missing_fields: Vec::new(),
        }],
    }
}

#[test]
fn missing_required_arg_forces_clarification_even_in_observe_mode() {
    let verify_result = verify_result_with_issue(
        crate::verifier::VerifyMode::ObserveOnly,
        crate::verifier::VerifyIssueKind::MissingRequiredArg,
    );

    assert!(verifier_gate_needs_clarification(&verify_result));
    assert!(verifier_gate_should_stop_round(&verify_result));
    assert_eq!(
        verifier_gate_missing_slots(&verify_result),
        vec!["required_execution_argument".to_string()]
    );
}

#[test]
fn route_clarify_issue_forces_clarification_even_in_observe_mode() {
    let verify_result = verify_result_with_issue(
        crate::verifier::VerifyMode::ObserveOnly,
        crate::verifier::VerifyIssueKind::RouteClarifyRequired,
    );

    assert!(verifier_gate_needs_clarification(&verify_result));
    assert!(verifier_gate_should_stop_round(&verify_result));
    assert_eq!(
        verifier_gate_missing_slots(&verify_result),
        vec!["execution_target_or_boundary".to_string()]
    );
}

#[test]
fn capability_unavailable_does_not_force_clarify_in_observe_mode() {
    let verify_result = verify_result_with_issue(
        crate::verifier::VerifyMode::ObserveOnly,
        crate::verifier::VerifyIssueKind::CapabilityUnavailable,
    );

    assert!(!verifier_gate_needs_clarification(&verify_result));
    assert!(!verifier_gate_should_stop_round(&verify_result));
}
