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
        capability_resolutions: Vec::new(),
    }
}

fn run_cmd_confirmation_verify_result(literal_user_command: bool) -> crate::verifier::VerifyResult {
    let mut args = serde_json::json!({
        "command": "python3 -c 'open(\"src/lib.rs\",\"w\").write(\"ok\")'"
    });
    if literal_user_command {
        args[crate::agent_engine::CLAWD_LITERAL_COMMAND_ARG] = serde_json::json!(true);
    }
    crate::verifier::VerifyResult {
        mode: crate::verifier::VerifyMode::Enforce,
        approved: true,
        blocked_reason: None,
        shadow_blocked_reason: Some("ConfirmationRequired".to_string()),
        permission_decision: serde_json::json!({
            "schema_version": 1,
            "owner_layer": "plan_verifier",
        }),
        approved_steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "run_cmd".to_string(),
            args,
            depends_on: Vec::new(),
            why: String::new(),
        }],
        needs_confirmation: true,
        rewritten_steps: Vec::new(),
        issues: vec![crate::verifier::VerifyIssue {
            step_id: "step_1".to_string(),
            kind: crate::verifier::VerifyIssueKind::ConfirmationRequired,
            detail: "skill `run_cmd` may require explicit confirmation".to_string(),
            missing_fields: Vec::new(),
        }],
        capability_resolutions: Vec::new(),
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
fn repairability_predicate_matches_bounded_replan_contract() {
    assert!(plan_verifier_rejection_is_repairable(&verify_result(&[
        crate::verifier::VerifyIssueKind::InvalidArgumentValue,
    ])));
    assert!(!plan_verifier_rejection_is_repairable(&verify_result(&[
        crate::verifier::VerifyIssueKind::SandboxPolicyDenied,
    ])));
}

#[test]
fn informational_verifier_issue_does_not_block_argument_replan() {
    let verify_result = verify_result(&[
        crate::verifier::VerifyIssueKind::DefaultCreationTargetApplied,
        crate::verifier::VerifyIssueKind::InvalidArgumentValue,
    ]);

    assert!(plan_verifier_rejection_is_repairable(&verify_result));
    let signal = planner_repair_signal(&verify_result).expect("machine repair signal");
    let issues = signal["issues"].as_array().expect("issue array");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["verify_issue_kind"], "InvalidArgumentValue");
}

#[test]
fn informational_verifier_issue_alone_does_not_request_replan() {
    assert!(!plan_verifier_rejection_is_repairable(&verify_result(&[
        crate::verifier::VerifyIssueKind::DefaultCreationTargetApplied,
    ])));
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
fn missing_planner_argument_enters_bounded_replan() {
    let mut loop_state = LoopState::default();
    let outcome = recover_plan_verifier_rejection(
        &mut loop_state,
        &verify_result(&[crate::verifier::VerifyIssueKind::MissingRequiredArg]),
    )
    .expect("planner-generated missing argument should be repairable");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(loop_state.has_recoverable_failure_context);
    assert!(loop_state.history_compact.iter().any(|entry| {
        entry.contains("plan_verifier_replan_required")
            && entry.contains("\"verify_issue_kind\":\"MissingRequiredArg\"")
    }));
}

#[test]
fn explicit_boundary_clarification_never_enters_planner_repair() {
    let mut loop_state = LoopState::default();
    let outcome = recover_plan_verifier_rejection(
        &mut loop_state,
        &verify_result(&[crate::verifier::VerifyIssueKind::BoundaryClarifyRequired]),
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

#[test]
fn planner_generated_run_cmd_confirmation_gets_one_scoped_capability_replan() {
    let mut loop_state = LoopState::default();
    let verify_result = run_cmd_confirmation_verify_result(false);

    let first =
        recover_run_cmd_confirmation_with_scoped_capability_replan(&mut loop_state, &verify_result)
            .expect("first confirmation boundary should replan");
    assert_eq!(
        first.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert_eq!(
        first.next_goal_hint.as_deref(),
        Some("replan_with_scoped_capabilities")
    );
    let signal: serde_json::Value = serde_json::from_str(
        loop_state
            .last_output
            .as_deref()
            .expect("machine replan signal"),
    )
    .expect("valid signal");
    assert_eq!(
        signal["status_code"],
        "plan_verifier_scoped_capability_replan_required"
    );
    assert_eq!(signal["preferred_capabilities"][0], "filesystem.make_dir");
    assert_eq!(signal["confirmation_policy_unchanged"], true);

    assert!(recover_run_cmd_confirmation_with_scoped_capability_replan(
        &mut loop_state,
        &verify_result,
    )
    .is_none());
}

#[test]
fn literal_user_run_cmd_confirmation_never_enters_scoped_capability_replan() {
    let mut loop_state = LoopState::default();
    let verify_result = run_cmd_confirmation_verify_result(true);

    assert!(recover_run_cmd_confirmation_with_scoped_capability_replan(
        &mut loop_state,
        &verify_result,
    )
    .is_none());
    assert!(loop_state.history_compact.is_empty());
}

#[test]
fn mixed_confirmation_and_sandbox_denial_remains_fail_closed() {
    let mut loop_state = LoopState::default();
    let mut verify_result = run_cmd_confirmation_verify_result(false);
    verify_result.approved = false;
    verify_result.blocked_reason = Some("SandboxPolicyDenied".to_string());
    verify_result.issues.push(crate::verifier::VerifyIssue {
        step_id: "step_1".to_string(),
        kind: crate::verifier::VerifyIssueKind::SandboxPolicyDenied,
        detail: "reason_code=sandbox_backend_unavailable".to_string(),
        missing_fields: Vec::new(),
    });

    assert!(recover_run_cmd_confirmation_with_scoped_capability_replan(
        &mut loop_state,
        &verify_result,
    )
    .is_none());
    assert!(loop_state.history_compact.is_empty());
}
