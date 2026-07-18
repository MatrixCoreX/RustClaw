use super::*;

fn verify_result(kinds: &[crate::verifier::VerifyIssueKind]) -> crate::verifier::VerifyResult {
    crate::verifier::VerifyResult {
        mode: crate::verifier::VerifyMode::Enforce,
        approved: false,
        blocked_reason: Some("verification_failed".to_string()),
        shadow_blocked_reason: None,
        permission_decision: serde_json::json!({
            "schema_version": 1,
            "owner_layer": "plan_verifier",
        }),
        approved_steps: Vec::new(),
        needs_confirmation: false,
        rewritten_steps: Vec::new(),
        issues: kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| crate::verifier::VerifyIssue {
                step_id: format!("step_{}", index + 1),
                kind: *kind,
                detail:
                    "error_code=invalid_argument_value field=unknown constraint=declared_property"
                        .to_string(),
                missing_fields: Vec::new(),
            })
            .collect(),
    }
}

#[test]
fn invalid_planner_arguments_enter_bounded_replan() {
    let mut loop_state = LoopState::default();
    let outcome = recover_plan_verifier_rejection(
        &mut loop_state,
        &verify_result(&[crate::verifier::VerifyIssueKind::InvalidArgumentValue]),
    )
    .expect("invalid planner argument should be repairable");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(loop_state.has_recoverable_failure_context);
    assert!(loop_state.history_compact.iter().any(|entry| {
        entry.contains("plan_verifier_replan_required") && entry.contains("declared_property")
    }));
}

#[test]
fn permission_denial_never_enters_planner_repair() {
    let mut loop_state = LoopState::default();
    let outcome = recover_plan_verifier_rejection(
        &mut loop_state,
        &verify_result(&[crate::verifier::VerifyIssueKind::SandboxPolicyDenied]),
    );

    assert!(outcome.is_none());
    assert!(loop_state.history_compact.is_empty());
}

#[test]
fn missing_user_argument_stays_on_clarification_boundary() {
    let mut loop_state = LoopState::default();
    let outcome = recover_plan_verifier_rejection(
        &mut loop_state,
        &verify_result(&[crate::verifier::VerifyIssueKind::MissingRequiredArg]),
    );

    assert!(outcome.is_none());
    assert!(loop_state.history_compact.is_empty());
}

#[test]
fn mixed_repairable_and_policy_issues_do_not_bypass_policy() {
    let mut loop_state = LoopState::default();
    let outcome = recover_plan_verifier_rejection(
        &mut loop_state,
        &verify_result(&[
            crate::verifier::VerifyIssueKind::InvalidArgumentValue,
            crate::verifier::VerifyIssueKind::SandboxPolicyDenied,
        ]),
    );

    assert!(outcome.is_none());
    assert!(loop_state.history_compact.is_empty());
}
