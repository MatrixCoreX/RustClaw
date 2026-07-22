use serde_json::json;

use super::super::{LoopState, RoundOutcome};

const SCOPED_CAPABILITY_REPLAN_ATTEMPTED: &str = "agent_loop.scoped_capability_replan_attempted";

fn issue_is_planner_repairable(kind: crate::verifier::VerifyIssueKind) -> bool {
    matches!(
        kind,
        crate::verifier::VerifyIssueKind::SkillNotVisible
            | crate::verifier::VerifyIssueKind::CapabilityUnavailable
            | crate::verifier::VerifyIssueKind::InvalidArgumentValue
            | crate::verifier::VerifyIssueKind::UnresolvedTemplateArg
            | crate::verifier::VerifyIssueKind::InvalidDependsOn
            | crate::verifier::VerifyIssueKind::PrimaryFallbackConflict
            | crate::verifier::VerifyIssueKind::RecipeInspectBeforeMutateRequired
            | crate::verifier::VerifyIssueKind::RecipeValidationAfterMutateRequired
            | crate::verifier::VerifyIssueKind::RecipeTargetScopeRequired
    )
}

pub(in crate::agent_engine) fn plan_verifier_rejection_is_repairable(
    verify_result: &crate::verifier::VerifyResult,
) -> bool {
    let mut blocking_issues = verify_result
        .issues
        .iter()
        .filter(|issue| crate::verifier::issue_blocks_in_enforce(issue.kind))
        .peekable();
    verify_result.mode == crate::verifier::VerifyMode::Enforce
        && !verify_result.approved
        && !verify_result.needs_confirmation
        && blocking_issues.peek().is_some()
        && blocking_issues.all(|issue| issue_is_planner_repairable(issue.kind))
}

fn planner_repair_signal(
    verify_result: &crate::verifier::VerifyResult,
) -> Option<serde_json::Value> {
    if !plan_verifier_rejection_is_repairable(verify_result) {
        return None;
    }

    Some(json!({
        "schema_version": 1,
        "status_code": "plan_verifier_replan_required",
        "issues": verify_result
            .issues
            .iter()
            .filter(|issue| crate::verifier::issue_blocks_in_enforce(issue.kind))
            .map(|issue| json!({
                "step_id": issue.step_id,
                "verify_issue_kind": issue.kind.as_str(),
                "status_code": issue.kind.status_code(),
                "message_key": issue.kind.message_key(),
                "missing_fields": issue.missing_fields,
                "machine_detail": crate::truncate_for_agent_trace(&issue.detail),
            }))
            .collect::<Vec<_>>(),
    }))
}

pub(super) fn recover_plan_verifier_rejection(
    loop_state: &mut LoopState,
    verify_result: &crate::verifier::VerifyResult,
) -> Option<RoundOutcome> {
    let signal = planner_repair_signal(verify_result)?;
    let serialized = serde_json::to_string(&signal).ok()?;
    loop_state.history_compact.push(serialized.clone());
    loop_state.last_output = Some(serialized.clone());
    loop_state
        .output_vars
        .insert("agent_loop.verifier_replan_signal".to_string(), serialized);
    loop_state.has_recoverable_failure_context = true;

    Some(RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("replan_from_verifier_signal".to_string()),
        no_progress: false,
    })
}

fn planner_generated_run_cmd_confirmation_can_replan(
    verify_result: &crate::verifier::VerifyResult,
) -> bool {
    if verify_result.mode != crate::verifier::VerifyMode::Enforce
        || !verify_result.approved
        || !verify_result.needs_confirmation
        || verify_result.issues.is_empty()
        || !verify_result
            .issues
            .iter()
            .all(|issue| issue.kind == crate::verifier::VerifyIssueKind::ConfirmationRequired)
    {
        return false;
    }

    verify_result.issues.iter().all(|issue| {
        verify_result
            .approved_steps
            .iter()
            .find(|step| step.step_id == issue.step_id)
            .is_some_and(|step| {
                matches!(step.action_type.as_str(), "call_tool" | "call_skill")
                    && matches!(step.skill.as_str(), "run_cmd" | "system.run_command")
                    && !step
                        .args
                        .get(crate::agent_engine::CLAWD_LITERAL_COMMAND_ARG)
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
            })
    })
}

pub(super) fn recover_run_cmd_confirmation_with_scoped_capability_replan(
    loop_state: &mut LoopState,
    verify_result: &crate::verifier::VerifyResult,
) -> Option<RoundOutcome> {
    if !planner_generated_run_cmd_confirmation_can_replan(verify_result)
        || loop_state
            .output_vars
            .contains_key(SCOPED_CAPABILITY_REPLAN_ATTEMPTED)
    {
        return None;
    }

    let signal = json!({
        "schema_version": 1,
        "status_code": "plan_verifier_scoped_capability_replan_required",
        "repair_code": "replace_shell_workspace_mutation_with_scoped_capabilities",
        "blocked_action_kind": "run_cmd",
        "preferred_capabilities": [
            "filesystem.make_dir",
            "filesystem.write_text",
            "workspace.apply_patch"
        ],
        "confirmation_policy_unchanged": true,
        "issues": verify_result
            .issues
            .iter()
            .map(|issue| json!({
                "step_id": issue.step_id,
                "verify_issue_kind": issue.kind.as_str(),
                "status_code": issue.kind.status_code(),
            }))
            .collect::<Vec<_>>(),
    });
    let serialized = serde_json::to_string(&signal).ok()?;
    loop_state.history_compact.push(serialized.clone());
    loop_state.last_output = Some(serialized.clone());
    loop_state
        .output_vars
        .insert(SCOPED_CAPABILITY_REPLAN_ATTEMPTED.to_string(), serialized);
    loop_state.has_recoverable_failure_context = true;

    Some(RoundOutcome {
        executed_actions: 0,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("replan_with_scoped_capabilities".to_string()),
        no_progress: false,
    })
}

#[cfg(test)]
#[path = "loop_control_plan_verifier_recovery_tests.rs"]
mod tests;
