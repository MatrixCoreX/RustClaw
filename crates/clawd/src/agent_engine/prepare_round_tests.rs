use super::{
    planner_user_text, production_verify_mode, verifier_confirmation_gate_requires_checkpoint,
    verifier_gate_missing_slots, verifier_gate_needs_clarification,
    verifier_gate_should_stop_round,
};

#[test]
fn planner_prefers_raw_current_request_over_pre_route_rewrite() {
    let context = crate::agent_engine::AgentRunContext {
        original_user_request: Some("raw current request".to_string()),
        user_request: Some("pre-route semantic rewrite".to_string()),
        ..Default::default()
    };

    assert_eq!(
        planner_user_text(Some(&context), "fallback request"),
        "raw current request"
    );
}

#[test]
fn production_agent_loop_always_enforces_plan_verification() {
    assert_eq!(
        production_verify_mode(),
        crate::verifier::VerifyMode::Enforce
    );
}

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
        capability_resolutions: Vec::new(),
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
        crate::verifier::VerifyIssueKind::BoundaryClarifyRequired,
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

#[test]
fn confirmation_requirement_stops_production_round_before_execution() {
    let mut verify_result = verify_result_with_issue(
        production_verify_mode(),
        crate::verifier::VerifyIssueKind::ConfirmationRequired,
    );
    verify_result.approved = true;
    verify_result.needs_confirmation = true;

    assert!(verifier_gate_should_stop_round(&verify_result));
    assert!(verifier_confirmation_gate_requires_checkpoint(
        &verify_result
    ));
}
