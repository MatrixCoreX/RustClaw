use serde_json::{json, Value};

use super::LoopState;

const MAX_ATTEMPT_LEDGER_STEPS: usize = 10;

#[derive(Debug, Clone, Default)]
pub(crate) struct AttemptLedgerEntry {
    pub(super) attempt_id: String,
    pub(super) tool_or_skill: String,
    pub(super) args_summary: String,
    pub(super) status: String,
    pub(super) observed_output: String,
    pub(super) error_kind: Option<String>,
    pub(super) retryable: bool,
    pub(super) why_not_satisfied: String,
    pub(super) retry_instruction: Option<String>,
    pub(super) avoid_repeating: String,
    pub(super) contract_policy: Option<Value>,
}

pub(super) fn record_attempt(
    loop_state: &mut LoopState,
    tool_or_skill: &str,
    args_summary: &str,
    status: crate::executor::StepExecutionStatus,
    observed_output: &str,
    error_kind: Option<&str>,
    why_not_satisfied: &str,
) {
    record_attempt_with_retry_instruction(
        loop_state,
        tool_or_skill,
        args_summary,
        status,
        observed_output,
        error_kind,
        why_not_satisfied,
        None,
    );
}

pub(super) fn record_attempt_with_retry_instruction(
    loop_state: &mut LoopState,
    tool_or_skill: &str,
    args_summary: &str,
    status: crate::executor::StepExecutionStatus,
    observed_output: &str,
    error_kind: Option<&str>,
    why_not_satisfied: &str,
    retry_instruction: Option<&str>,
) {
    let attempt_id = format!("a{}", loop_state.attempt_ledger_entries.len() + 1);
    let error_kind = error_kind
        .map(str::to_string)
        .or_else(|| structured_error_kind(why_not_satisfied))
        .or_else(|| structured_error_kind(observed_output));
    let retryable = retryable_from_status(status, error_kind.as_deref());
    let retry_instruction = retry_instruction
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::truncate_for_agent_trace);
    let contract_policy = contract_policy_from_structured_error(why_not_satisfied)
        .or_else(|| contract_policy_from_structured_error(observed_output));
    loop_state.attempt_ledger_entries.push(AttemptLedgerEntry {
        attempt_id,
        tool_or_skill: tool_or_skill.trim().to_string(),
        args_summary: if args_summary.trim().is_empty() {
            "(empty)".to_string()
        } else {
            crate::truncate_for_agent_trace(args_summary.trim())
        },
        status: status.as_str().to_string(),
        observed_output: crate::truncate_for_agent_trace(observed_output.trim()),
        error_kind: error_kind.clone(),
        retryable,
        why_not_satisfied: if why_not_satisfied.trim().is_empty() {
            why_not_satisfied_from_status(status, observed_output, "")
        } else {
            crate::truncate_for_agent_trace(why_not_satisfied.trim())
        },
        retry_instruction,
        avoid_repeating: avoid_repeating_hint(status, error_kind.as_deref()).to_string(),
        contract_policy,
    });
}

pub(super) fn build_attempt_ledger_compact(loop_state: &LoopState) -> String {
    if !loop_state.attempt_ledger_entries.is_empty() {
        let mut entries = loop_state
            .attempt_ledger_entries
            .iter()
            .rev()
            .take(MAX_ATTEMPT_LEDGER_STEPS)
            .map(attempt_entry_json)
            .collect::<Vec<_>>();
        entries.reverse();
        return serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string());
    }
    if loop_state.executed_step_results.is_empty() {
        return "(empty)".to_string();
    }
    let mut entries = loop_state
        .executed_step_results
        .iter()
        .rev()
        .take(MAX_ATTEMPT_LEDGER_STEPS)
        .enumerate()
        .map(|(idx, step)| {
            let error_text = step.error.as_deref().unwrap_or_default();
            let output_text = step.output.as_deref().unwrap_or_default();
            let error_kind = structured_error_kind(error_text);
            let contract_policy = contract_policy_from_structured_error(error_text)
                .or_else(|| contract_policy_from_structured_error(output_text));
            json!({
                "attempt_id": format!("a{}", idx + 1),
                "tool_or_skill": step.skill,
                "status": step.status.as_str(),
                "args_summary": "not_recorded_in_step_result",
                "observed_output": crate::truncate_for_agent_trace(output_text),
                "error_kind": error_kind,
                "retryable": retryable_from_status(step.status, error_kind.as_deref()),
                "why_not_satisfied": why_not_satisfied_from_status(step.status, output_text, error_text),
                "retry_instruction": null,
                "avoid_repeating": avoid_repeating_hint(step.status, error_kind.as_deref()),
                "contract_policy": contract_policy,
            })
        })
        .collect::<Vec<_>>();
    entries.reverse();
    serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
}

fn attempt_entry_json(entry: &AttemptLedgerEntry) -> serde_json::Value {
    json!({
        "attempt_id": entry.attempt_id,
        "tool_or_skill": entry.tool_or_skill,
        "status": entry.status,
        "args_summary": entry.args_summary,
        "observed_output": entry.observed_output,
        "error_kind": entry.error_kind,
        "retryable": entry.retryable,
        "why_not_satisfied": entry.why_not_satisfied,
        "retry_instruction": entry.retry_instruction,
        "avoid_repeating": entry.avoid_repeating,
        "contract_policy": entry.contract_policy,
    })
}

fn structured_error_kind(error_text: &str) -> Option<String> {
    crate::skills::parse_structured_skill_error(error_text)
        .map(|structured| structured.error_kind)
        .or_else(|| (!error_text.trim().is_empty()).then_some("unclassified_error".to_string()))
}

fn contract_policy_from_structured_error(error_text: &str) -> Option<Value> {
    let structured = crate::skills::parse_structured_skill_error(error_text)?;
    if !matches!(
        structured.error_kind.as_str(),
        "contract_action_rejected" | "contract_arg_rejected"
    ) {
        return None;
    }
    let extra = structured.extra.as_ref()?;
    Some(json!({
        "error_kind": structured.error_kind.as_str(),
        "decision": extra.get("decision").and_then(Value::as_str),
        "action": extra.get("action").and_then(Value::as_str),
        "contract_match": extra.get("contract_match").and_then(Value::as_str),
        "preferred_actions": extra.get("preferred_actions").cloned(),
        "missing_target_args": extra.get("missing_target_args").cloned(),
        "expected_target_args": extra.get("expected_target_args").cloned(),
        "required_evidence": extra.get("required_evidence").cloned(),
        "final_answer_shape": extra.get("final_answer_shape").and_then(Value::as_str),
    }))
}

fn why_not_satisfied_from_status(
    status: crate::executor::StepExecutionStatus,
    output_text: &str,
    error_text: &str,
) -> String {
    match status {
        crate::executor::StepExecutionStatus::Ok => {
            let trimmed = output_text.trim();
            if trimmed.is_empty() {
                "step_completed_without_output".to_string()
            } else {
                "step_completed_with_observation".to_string()
            }
        }
        crate::executor::StepExecutionStatus::Error => {
            let trimmed = error_text.trim();
            if trimmed.is_empty() {
                "step_failed_without_error_text".to_string()
            } else {
                crate::truncate_for_agent_trace(trimmed)
            }
        }
    }
}

fn avoid_repeating_hint(
    status: crate::executor::StepExecutionStatus,
    error_kind: Option<&str>,
) -> &'static str {
    match status {
        crate::executor::StepExecutionStatus::Ok => {
            "do_not_repeat_unless_final_answer_needs_fresh_or_missing_evidence"
        }
        crate::executor::StepExecutionStatus::Error => match error_kind {
            Some("not_found") => {
                "do_not_retry_same_target; broaden_or_clarify_only_if_user_goal_requires_it"
            }
            Some("confirmed_not_found") => {
                "stop_and_report_confirmed_absence; do_not_retry_without_new_user_input"
            }
            Some("invalid_credentials") | Some("credential_missing") | Some("auth_failed") => {
                "stop_and_report_credential_problem; do_not_retry_until_credentials_change"
            }
            Some("policy_block") | Some("permission_denied") => {
                "do_not_bypass_policy; request_confirmation_or_stop_when_required"
            }
            Some("contract_action_rejected") => {
                "do_not_repeat_rejected_action; choose_contract_allowed_action_or_replan"
            }
            Some("contract_arg_rejected") => {
                "do_not_repeat_missing_target_binding; bind_target_or_ask_for_clarification"
            }
            _ => "do_not_repeat_same_tool_and_same_target; change_arguments_tool_or_plan",
        },
    }
}

fn retryable_from_status(
    status: crate::executor::StepExecutionStatus,
    error_kind: Option<&str>,
) -> bool {
    match status {
        crate::executor::StepExecutionStatus::Ok => false,
        crate::executor::StepExecutionStatus::Error => !matches!(
            error_kind,
            Some(
                "policy_block"
                    | "contract_action_rejected"
                    | "contract_arg_rejected"
                    | "unsafe_sql"
                    | "path_outside_workspace"
                    | "confirmation_required"
                    | "confirmed_not_found"
                    | "invalid_credentials"
                    | "credential_missing"
                    | "auth_failed"
                    | "invalid_input"
                    | "missing_input"
                    | "unsupported_action"
            )
        ),
    }
}

#[cfg(test)]
#[path = "attempt_ledger_tests.rs"]
mod tests;
