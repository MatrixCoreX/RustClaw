use serde_json::{json, Value};

use super::LoopState;
use crate::agent_runtime_contract::SubagentRole;

pub(super) const SUBAGENT_STOP_SIGNAL_INVALID_ROLE: &str = "subagent_invalid_role";
const MAX_SUBAGENT_CONTEXT_REFS: usize = 16;
const MAX_SUBAGENT_CAPABILITIES: usize = 32;
const MAX_SUBAGENT_RESULT_CONTRACT_KEYS: usize = 16;

pub(super) fn record_subagent_action(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    role: &str,
    objective: &str,
    context_refs: &[String],
    options: SubagentActionOptions,
) -> Option<&'static str> {
    let role_token = role.trim();
    let Some(role) = SubagentRole::parse_token(role_token) else {
        loop_state.task_observations.push(json!({
            "schema_version": 1,
            "owner_layer": "subagent_runtime",
            "status": "rejected",
            "error_code": "subagent_role_not_allowed",
            "role": role_token,
            "allowed_roles": SubagentRole::all_tokens(),
            "write_enabled": false,
            "external_publish_enabled": false,
            "global_step": global_step,
            "step_in_round": step_in_round,
            "round_no": loop_state.round_no,
        }));
        return Some(SUBAGENT_STOP_SIGNAL_INVALID_ROLE);
    };
    let child_run_id = format!(
        "subagent:{}:{}:{}",
        loop_state.round_no,
        step_in_round,
        role.as_token()
    );
    let context_refs = safe_context_refs(context_refs);
    let allowed_capabilities = safe_machine_token_list(&options.allowed_capabilities);
    let context_ref_count = context_refs.len();
    let allowed_capability_count = allowed_capabilities.len();
    let role_metadata = role_metadata_summary(role);
    let budget_summary = subagent_budget_summary(options.budget.as_ref());
    let timeout_policy = subagent_timeout_policy(&budget_summary);
    let cancellation_policy = subagent_cancellation_policy(&timeout_policy);
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "subagent_runtime",
        "status": "accepted",
        "execution_mode": "inline_readonly_child_run",
        "child_run_id": child_run_id.as_str(),
        "role": role.as_token(),
        "role_metadata": role_metadata,
        "objective_present": !objective.trim().is_empty(),
        "objective_char_count": objective.chars().count(),
        "context_refs": &context_refs,
        "context_ref_count": context_ref_count,
        "allowed_capabilities": &allowed_capabilities,
        "allowed_capability_count": allowed_capability_count,
        "budget": budget_summary,
        "timeout_policy": timeout_policy,
        "cancellation_policy": cancellation_policy,
        "parent_task_ref": machine_ref_or_empty(options.parent_task_id.as_deref().unwrap_or_default()),
        "context_slice": context_slice_summary(options.context_slice.as_ref()),
        "result_contract": result_contract_summary(options.result_contract.as_ref()),
        "child_request": child_request_envelope(
            child_run_id.as_str(),
            role,
            context_ref_count,
            allowed_capability_count,
            options.budget.as_ref(),
        ),
        "scheduler": {
            "status": "inline_completed",
            "reason_code": "readonly_subagent_inline_execution",
            "lease_required": false,
            "checkpoint_required": false,
            "child_request_ref": child_run_id.as_str(),
        },
        "merge_contract": {
            "strategy": "append_child_trace_summary",
            "parent_trace_event_type": "subagent",
            "child_trace_merge_status": "merged",
            "result_status": "completed",
            "failure_isolated": true,
        },
        "child_run_summary": {
            "child_run_id": child_run_id.as_str(),
            "status": "completed",
            "result_status": "completed",
            "trace_merge_status": "merged",
            "role": role.as_token(),
            "context_ref_count": context_ref_count,
            "allowed_capability_count": allowed_capability_count,
            "role_family": role.family_token(),
            "write_enabled": false,
            "external_publish_enabled": false,
            "failure_isolated": true
        },
        "child_result": {
            "schema_version": 1,
            "status": "completed",
            "result_status": "completed",
            "outcome_code": "subagent_inline_readonly_completed",
            "role": role.as_token(),
            "role_family": role.family_token(),
            "context_ref_count": context_ref_count,
            "allowed_capability_count": allowed_capability_count,
            "result_contract_present": options.result_contract.is_some(),
            "result_contract_required": role.result_contract_required(),
            "write_enabled": false,
            "external_publish_enabled": false,
            "failure_isolated": true
        },
        "write_enabled": false,
        "external_publish_enabled": false,
        "failure_isolated": true,
        "global_step": global_step,
        "step_in_round": step_in_round,
        "round_no": loop_state.round_no,
    }));
    None
}

pub(super) fn record_subagent_action_from_args(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
) -> Option<&'static str> {
    let role = args.get("role").and_then(Value::as_str).unwrap_or_default();
    let objective = args
        .get("objective")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let context_refs = args
        .get("context_refs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .or_else(|| context_refs_from_context_slice(args.get("context_slice")))
        .unwrap_or_default();
    let options = SubagentActionOptions {
        allowed_capabilities: args
            .get("allowed_capabilities")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        budget: args.get("budget").cloned(),
        parent_task_id: args
            .get("parent_task_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        context_slice: args.get("context_slice").cloned(),
        result_contract: args.get("result_contract").cloned(),
    };
    record_subagent_action(
        loop_state,
        global_step,
        step_in_round,
        role,
        objective,
        &context_refs,
        options,
    )
}

pub(super) struct SubagentActionOptions {
    allowed_capabilities: Vec<String>,
    budget: Option<Value>,
    parent_task_id: Option<String>,
    context_slice: Option<Value>,
    result_contract: Option<Value>,
}

impl Default for SubagentActionOptions {
    fn default() -> Self {
        Self {
            allowed_capabilities: Vec::new(),
            budget: None,
            parent_task_id: None,
            context_slice: None,
            result_contract: None,
        }
    }
}

fn context_refs_from_context_slice(context_slice: Option<&Value>) -> Option<Vec<String>> {
    let context_slice = context_slice?.as_object()?;
    for key in ["refs", "evidence_refs", "context_refs"] {
        if let Some(items) = context_slice.get(key).and_then(Value::as_array) {
            return Some(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            );
        }
    }
    None
}

fn safe_context_refs(context_refs: &[String]) -> Vec<Value> {
    context_refs
        .iter()
        .take(MAX_SUBAGENT_CONTEXT_REFS)
        .map(|value| {
            let trimmed = value.trim();
            json!({
                "present": !trimmed.is_empty(),
                "char_count": trimmed.chars().count(),
                "ref": machine_ref_or_empty(trimmed),
            })
        })
        .collect()
}

fn safe_machine_token_list(values: &[String]) -> Vec<Value> {
    values
        .iter()
        .take(MAX_SUBAGENT_CAPABILITIES)
        .map(|value| {
            let token = machine_ref_or_empty(value.trim());
            json!({
                "present": !value.trim().is_empty(),
                "token": token,
            })
        })
        .collect()
}

fn subagent_budget_summary(budget: Option<&Value>) -> Value {
    let Some(budget) = budget.and_then(Value::as_object) else {
        return json!({
            "present": false,
        });
    };
    json!({
        "present": true,
        "max_rounds": budget.get("max_rounds").and_then(Value::as_u64),
        "max_tool_calls": budget.get("max_tool_calls").and_then(Value::as_u64),
        "max_context_chars": budget.get("max_context_chars").and_then(Value::as_u64),
        "timeout_ms": budget.get("timeout_ms").and_then(Value::as_u64),
    })
}

fn role_metadata_summary(role: SubagentRole) -> Value {
    json!({
        "schema_version": 1,
        "role": role.as_token(),
        "role_family": role.family_token(),
        "default_scope": role.default_scope_token(),
        "tool_permission_profile": "read_only",
        "parallel_eligible": true,
        "result_contract_required": role.result_contract_required(),
        "write_enabled": false,
        "external_publish_enabled": false,
    })
}

fn subagent_timeout_policy(budget_summary: &Value) -> Value {
    let timeout_ms = budget_summary
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0);
    json!({
        "schema_version": 1,
        "policy": "bounded",
        "timeout_ms": timeout_ms,
        "timeout_required": true,
        "timeout_source": if timeout_ms.is_some() { "budget.timeout_ms" } else { "parent_loop_default" },
        "terminal_status_on_timeout": "timeout",
    })
}

fn subagent_cancellation_policy(timeout_policy: &Value) -> Value {
    json!({
        "schema_version": 1,
        "cancellable": true,
        "cancel_status": "cancelled",
        "cancel_scope": "child_run",
        "parent_failure_policy": "isolate_optional_child_failure",
        "timeout_policy_ref": timeout_policy.get("policy").and_then(Value::as_str),
    })
}

fn context_slice_summary(context_slice: Option<&Value>) -> Value {
    let Some(context_slice) = context_slice.and_then(Value::as_object) else {
        return json!({
            "present": false,
        });
    };
    let refs = context_refs_from_context_slice(Some(&Value::Object(context_slice.clone())))
        .unwrap_or_default();
    json!({
        "present": true,
        "ref_count": refs.len(),
        "refs": safe_context_refs(&refs),
        "max_context_chars": context_slice.get("max_context_chars").and_then(Value::as_u64),
    })
}

fn child_request_envelope(
    child_run_id: &str,
    role: SubagentRole,
    context_ref_count: usize,
    allowed_capability_count: usize,
    budget: Option<&Value>,
) -> Value {
    let budget_summary = subagent_budget_summary(budget);
    let timeout_policy = subagent_timeout_policy(&budget_summary);
    json!({
        "schema_version": 1,
        "request_ref": child_run_id,
        "role": role.as_token(),
        "role_metadata": role_metadata_summary(role),
        "state": "completed",
        "execution_mode": "inline_readonly_child_run",
        "context_ref_count": context_ref_count,
        "allowed_capability_count": allowed_capability_count,
        "budget": budget_summary,
        "timeout_policy": timeout_policy,
        "cancellation_policy": subagent_cancellation_policy(&timeout_policy),
        "write_enabled": false,
        "external_publish_enabled": false,
        "failure_isolated": true,
    })
}

fn result_contract_summary(result_contract: Option<&Value>) -> Value {
    match result_contract {
        Some(Value::String(token)) => json!({
            "present": true,
            "kind": "token",
            "token": machine_ref_or_empty(token.trim()),
        }),
        Some(Value::Object(map)) => {
            let keys = map
                .keys()
                .take(MAX_SUBAGENT_RESULT_CONTRACT_KEYS)
                .map(|key| {
                    json!({
                        "key": machine_ref_or_empty(key),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "present": true,
                "kind": "object",
                "key_count": map.len(),
                "keys": keys,
            })
        }
        Some(_) => json!({
            "present": true,
            "kind": "unsupported",
        }),
        None => json!({
            "present": false,
        }),
    }
}

fn machine_ref_or_empty(value: &str) -> &str {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/' | '#'))
    {
        value
    } else {
        ""
    }
}

#[cfg(test)]
#[path = "subagent_runtime_tests.rs"]
mod tests;
