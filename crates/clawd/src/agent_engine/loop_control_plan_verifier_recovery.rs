use serde_json::{json, Value};

use super::super::{LoopState, RoundOutcome};

const SCOPED_CAPABILITY_REPLAN_ATTEMPTED: &str = "agent_loop.scoped_capability_replan_attempted";
const VERIFIER_REJECTION_FINGERPRINT: &str = "agent_loop.verifier_rejection_fingerprint";
const VERIFIER_REJECTION_REPEAT_COUNT: &str = "agent_loop.verifier_rejection_repeat_count";

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                if let Some(item) = map.get(key) {
                    sorted.insert(key.clone(), canonical_value(item));
                }
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

fn rejected_step_fingerprint(step: &crate::PlanStep) -> String {
    crate::evidence_policy::fnv1a_hex(&format!(
        "{}\n{}\n{}",
        step.action_type.trim(),
        step.skill.trim(),
        canonical_value(&step.args)
    ))
}

fn verifier_rejection_fingerprint(
    plan_result: &crate::PlanResult,
    verify_result: &crate::verifier::VerifyResult,
) -> String {
    let issues = verify_result
        .issues
        .iter()
        .filter(|issue| crate::verifier::issue_blocks_in_enforce(issue.kind))
        .map(|issue| {
            let rejected_step = plan_result
                .steps
                .iter()
                .find(|step| step.step_id == issue.step_id)
                .map(rejected_step_fingerprint);
            json!({
                "step_id": issue.step_id,
                "verify_issue_kind": issue.kind.as_str(),
                "status_code": issue.kind.status_code(),
                "missing_fields": issue.missing_fields,
                "machine_detail": issue.detail,
                "rejected_step_fingerprint": rejected_step,
            })
        })
        .collect::<Vec<_>>();
    crate::evidence_policy::fnv1a_hex(&canonical_value(&Value::Array(issues)).to_string())
}

fn issue_is_planner_repairable(kind: crate::verifier::VerifyIssueKind) -> bool {
    matches!(
        kind,
        crate::verifier::VerifyIssueKind::SkillNotVisible
            | crate::verifier::VerifyIssueKind::CapabilityUnavailable
            | crate::verifier::VerifyIssueKind::MissingRequiredArg
            | crate::verifier::VerifyIssueKind::InvalidArgumentValue
            | crate::verifier::VerifyIssueKind::UnresolvedTemplateArg
            | crate::verifier::VerifyIssueKind::InvalidDependsOn
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
    plan_result: &crate::PlanResult,
    verify_result: &crate::verifier::VerifyResult,
) -> Option<serde_json::Value> {
    if !plan_verifier_rejection_is_repairable(verify_result) {
        return None;
    }

    Some(json!({
        "schema_version": 1,
        "status_code": "plan_verifier_replan_required",
        "do_not_repeat_same_rejected_plan": true,
        "allowed_next_outcomes": [
            "materially_change_arguments",
            "request_missing_user_input",
            "respond_from_verifier_evidence"
        ],
        "issues": verify_result
            .issues
            .iter()
            .filter(|issue| crate::verifier::issue_blocks_in_enforce(issue.kind))
            .map(|issue| json!({
                "step_id": issue.step_id,
                "verify_issue_kind": issue.kind.as_str(),
                "status": "error",
                "error_code": issue.kind.status_code(),
                "status_code": issue.kind.status_code(),
                "message_key": issue.kind.message_key(),
                "retryable": false,
                "planner_repairable": true,
                "missing_fields": issue.missing_fields,
                "machine_detail": crate::truncate_for_agent_trace(&issue.detail),
                "forbidden_repeat_fingerprint": plan_result
                    .steps
                    .iter()
                    .find(|step| step.step_id == issue.step_id)
                    .map(rejected_step_fingerprint),
            }))
            .collect::<Vec<_>>(),
    }))
}

pub(super) fn recover_plan_verifier_rejection(
    loop_state: &mut LoopState,
    plan_result: &crate::PlanResult,
    verify_result: &crate::verifier::VerifyResult,
) -> Option<RoundOutcome> {
    let mut signal = planner_repair_signal(plan_result, verify_result)?;
    let fingerprint = verifier_rejection_fingerprint(plan_result, verify_result);
    let repeated = loop_state
        .output_vars
        .get(VERIFIER_REJECTION_FINGERPRINT)
        .is_some_and(|previous| previous == &fingerprint);
    let repeat_count = if repeated {
        loop_state
            .output_vars
            .get(VERIFIER_REJECTION_REPEAT_COUNT)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            .saturating_add(1)
    } else {
        1
    };
    loop_state.output_vars.insert(
        VERIFIER_REJECTION_FINGERPRINT.to_string(),
        fingerprint.clone(),
    );
    loop_state.output_vars.insert(
        VERIFIER_REJECTION_REPEAT_COUNT.to_string(),
        repeat_count.to_string(),
    );
    if repeated {
        signal["status_code"] = json!("plan_verifier_replan_repeated");
        signal["repair_code"] = json!("do_not_repeat_rejected_plan");
        signal["repeat_count"] = json!(repeat_count);
        signal["rejection_fingerprint"] = json!(fingerprint);
        signal["allowed_next_outcomes"] = json!([
            "materially_change_arguments",
            "request_missing_user_input",
            "respond_from_verifier_evidence"
        ]);
    }
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
        next_goal_hint: Some(if repeated {
            "do_not_repeat_rejected_plan".to_string()
        } else {
            "replan_from_verifier_signal".to_string()
        }),
        no_progress: repeated,
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
