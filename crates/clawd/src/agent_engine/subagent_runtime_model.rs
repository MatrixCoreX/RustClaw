use futures_util::future::join_all;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::{AppState, LoopState};
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
};
use crate::repo::child_tasks::{start_inline_child_task, ChildTaskParentContext};

const MAX_CHILD_ERROR_CHARS: usize = 512;

pub(super) async fn maybe_run_model_assisted_subagent(
    state: &AppState,
    task: &crate::ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
) {
    if args.get("children").and_then(Value::as_array).is_some() {
        maybe_run_model_assisted_subagent_batch(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            args,
        )
        .await;
        return;
    }
    let Some(child_input) = child_loop_input(loop_state, global_step, step_in_round, args) else {
        return;
    };
    let timeout_ms = child_input
        .pointer("/timeout_policy/timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(120_000)
        .clamp(1_000, 3_600_000);
    let child_result = run_readonly_child_agent_loop(state, task, &child_input, timeout_ms)
        .await
        .unwrap_or_else(|err| child_loop_error_result("subagent_child_loop_failed", &err));
    apply_model_assisted_child_result(loop_state, global_step, step_in_round, child_result);
}

async fn maybe_run_model_assisted_subagent_batch(
    state: &AppState,
    task: &crate::ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
) {
    if args.get("dry_run").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let Some(children) = args.get("children").and_then(Value::as_array) else {
        return;
    };
    let config = super::load_subagent_runtime_config(state);
    let requested_parallel = args
        .get("max_parallel")
        .and_then(Value::as_u64)
        .unwrap_or(config.max_parallel_readonly)
        .clamp(1, config.max_parallel_readonly) as usize;
    let default_timeout_ms = config.default_timeout_ms;
    let parallel_batch_id = latest_subagent_observation(loop_state, global_step, step_in_round)
        .and_then(|value| value.get("parallel_batch_id"))
        .and_then(Value::as_str)
        .unwrap_or("subagent-batch")
        .to_string();
    let futures = children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            let role = child.get("role")?.as_str()?.trim();
            let objective = child.get("objective")?.as_str()?.trim();
            let context_refs = child.get("context_refs")?.as_array()?;
            let allowed_capabilities = child.get("allowed_capabilities")?.as_array()?;
            if config.resolve_role(role).is_none()
                || objective.is_empty()
                || context_refs.is_empty()
                || allowed_capabilities.is_empty()
            {
                return None;
            }
            Some((index, child.clone(), role.to_string()))
        })
        .take(requested_parallel)
        .map(|(index, child, role)| {
            let child_run_id = format!(
                "{}:{}:{}",
                parallel_batch_id,
                index + 1,
                super::normalize_machine_token(&role)
            );
            async move {
                let timeout_ms = child
                    .pointer("/budget/timeout_ms")
                    .and_then(Value::as_u64)
                    .or(default_timeout_ms)
                    .unwrap_or(120_000)
                    .clamp(1_000, 3_600_000);
                let child_input = json!({
                    "schema_version": 1,
                    "role": role,
                    "objective": child.get("objective"),
                    "runtime_policy": {
                        "write_enabled": false,
                        "external_publish_enabled": false,
                        "tool_permission_profile": "read_only",
                    },
                    "context_refs": child.get("context_refs"),
                    "allowed_capabilities": child.get("allowed_capabilities"),
                    "budget": child.get("budget").cloned().unwrap_or_else(|| json!({})),
                    "timeout_policy": {
                        "policy": "bounded",
                        "timeout_ms": timeout_ms,
                    },
                    "result_contract": child
                        .get("result_contract")
                        .cloned()
                        .unwrap_or_else(|| json!({"output_format": "machine_json"})),
                });
                let result = run_readonly_child_agent_loop(state, task, &child_input, timeout_ms)
                    .await
                    .unwrap_or_else(|err| {
                        child_loop_error_result("subagent_child_loop_failed", &err)
                    });
                (
                    child_run_id,
                    child
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    result,
                )
            }
        })
        .collect::<Vec<_>>();
    if futures.is_empty() {
        return;
    }
    let results = join_all(futures).await;
    apply_model_assisted_batch_results(loop_state, global_step, step_in_round, results);
}

fn child_loop_input(
    loop_state: &LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
) -> Option<Value> {
    if args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let objective = args
        .get("objective")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let observation = latest_subagent_observation(loop_state, global_step, step_in_round)?;
    if !observation
        .get("context_evidence")?
        .get("present")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    Some(json!({
        "schema_version": 1,
        "role": observation.get("role").and_then(Value::as_str).unwrap_or("review"),
        "objective": objective,
        "runtime_policy": {
            "write_enabled": false,
            "external_publish_enabled": false,
            "tool_permission_profile": observation
                .pointer("/role_metadata/tool_permission_profile")
                .and_then(Value::as_str)
                .unwrap_or("read_only"),
        },
        "context_refs": observation.get("context_refs").cloned().unwrap_or_else(|| json!([])),
        "allowed_capabilities": args
            .get("allowed_capabilities")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "budget": observation.get("budget").cloned().unwrap_or_else(|| json!({})),
        "timeout_policy": observation
            .get("timeout_policy")
            .cloned()
            .unwrap_or_else(|| json!({})),
        "result_contract": args
            .get("result_contract")
            .cloned()
            .or_else(|| observation.get("result_contract").cloned())
            .unwrap_or_else(|| json!({"output_format": "machine_json"})),
    }))
}

fn latest_subagent_observation(
    loop_state: &LoopState,
    global_step: usize,
    step_in_round: usize,
) -> Option<&Value> {
    loop_state
        .task_observations
        .iter()
        .rev()
        .find(|observation| {
            observation
                .get("owner_layer")
                .and_then(Value::as_str)
                .is_some_and(|owner| owner == "subagent_runtime")
                && observation
                    .get("global_step")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == global_step)
                && observation
                    .get("step_in_round")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == step_in_round)
                && observation
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status == "accepted")
        })
}

pub(super) async fn run_readonly_child_agent_loop(
    state: &AppState,
    task: &crate::ClaimedTask,
    child_input: &Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    let objective = child_input
        .get("objective")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let role = child_input
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("review");
    let context_refs = child_input
        .get("context_refs")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let allowed_capabilities = child_input
        .get("allowed_capabilities")
        .cloned()
        .unwrap_or_else(|| json!([]));
    if !allowed_capabilities
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        return Err("subagent_child_allowed_capabilities_missing".to_string());
    }
    let result_contract = child_input
        .get("result_contract")
        .cloned()
        .unwrap_or_else(|| json!({"output_format": "machine_json"}));
    let child_ref = format!("{}:inline:{}", task.task_id, uuid::Uuid::new_v4().simple());
    let budget = inline_child_budget(child_input, timeout_ms);
    let runtime_config = super::load_subagent_runtime_config(state);
    let model_policy = runtime_config
        .resolve_role(role)
        .map(|definition| definition.model_policy.clone())
        .ok_or_else(|| "subagent_model_policy_role_missing".to_string())?;
    let spec = ChildTaskSpec {
        parent_task_id: task.task_id.clone(),
        child_task_id: child_ref.clone(),
        role: role.to_string(),
        scope: json!({
            "objective": objective,
            "context_refs": context_refs,
            "allowed_capabilities": allowed_capabilities,
            "model_policy": model_policy,
        }),
        permission_profile: ChildTaskPermissionProfile::ReadOnly,
        required: true,
        budget: budget.clone(),
        result_contract: result_contract.clone(),
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    };
    let parent = child_parent_context(task);
    let (child_task, child_payload) = start_inline_child_task(state, &parent, &spec)
        .map_err(|err| format!("subagent_inline_child_start_failed detail={err}"))?;
    let child_boundary = json!({
        "schema_version": 1,
        "owner_layer": "subagent_child_runtime",
        "status": "bound",
        "parent_task_id": task.task_id,
        "child_task_id": child_ref,
        "role": role,
        "context_refs": context_refs,
        "allowed_capabilities": allowed_capabilities,
        "budget": budget.to_json(),
        "runtime_policy": child_input.get("runtime_policy").cloned().unwrap_or_else(|| json!({})),
        "result_contract": result_contract,
        "resolved_model_policy": child_payload.get("resolved_model_policy"),
    });
    let child_goal = json!({
        "objective": objective,
        "required_output": {
            "owner_layer": "subagent_model_child",
            "output_format": "machine_json",
            "status": ["completed", "needs_more_evidence", "failed"],
            "required_fields": [
                "schema_version",
                "owner_layer",
                "output_format",
                "status",
                "role",
                "findings",
                "evidence_refs",
                "confidence"
            ]
        },
        "result_contract": result_contract,
        "budget": budget.to_json(),
        "completion_policy": {
            "respond_when_result_contract_satisfied": true,
            "repeat_completed_delegation": false,
        },
    })
    .to_string();
    let child_run = tokio::time::timeout(
        std::time::Duration::from_millis(budget.timeout_ms),
        Box::pin(crate::agent_engine::run_agent_with_tools(
            state,
            &child_task,
            &child_goal,
            objective,
            None,
            &[child_boundary],
        )),
    )
    .await;
    let reply = match child_run {
        Ok(Ok(reply)) => reply,
        Ok(Err(err)) => {
            let result = child_loop_error_result("subagent_child_loop_failed", &err);
            finalize_inline_child_failure(state, &child_task, &child_payload, &result, &err);
            return Ok(result);
        }
        Err(_) => {
            let error_code = "subagent_child_loop_timeout";
            let result = child_loop_error_result(error_code, error_code);
            finalize_inline_child_failure(state, &child_task, &child_payload, &result, error_code);
            return Ok(result);
        }
    };
    let raw_result = if reply.text.trim().is_empty() {
        reply
            .messages
            .last()
            .map(String::as_str)
            .unwrap_or_default()
    } else {
        reply.text.as_str()
    };
    if reply.should_fail_task {
        let result = child_loop_error_result(
            "subagent_child_loop_task_failed",
            reply.error_text.as_deref().unwrap_or(raw_result),
        );
        finalize_inline_child_failure(
            state,
            &child_task,
            &child_payload,
            &result,
            reply
                .error_text
                .as_deref()
                .unwrap_or("subagent_child_loop_task_failed"),
        );
        return Ok(result);
    }
    let result = parse_child_loop_result(raw_result, role, &context_refs, &result_contract);
    finalize_inline_child_success(state, &child_task, &child_payload, &result)?;
    Ok(result)
}

fn inline_child_budget(child_input: &Value, timeout_ms: u64) -> ChildTaskBudget {
    let budget = child_input.get("budget").unwrap_or(&Value::Null);
    ChildTaskBudget {
        max_rounds: budget
            .get("max_rounds")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            .max(1),
        max_tool_calls: budget
            .get("max_tool_calls")
            .and_then(Value::as_u64)
            .unwrap_or(16)
            .max(1),
        max_tokens: budget
            .get("max_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(1_000_000)
            .max(1),
        timeout_ms: timeout_ms.max(1_000),
    }
}

fn child_parent_context(task: &crate::ClaimedTask) -> ChildTaskParentContext {
    let execution_policy_stamp = serde_json::from_str::<Value>(&task.payload_json)
        .ok()
        .and_then(|payload| crate::task_execution_policy::policy_payload(&payload).cloned());
    ChildTaskParentContext {
        parent_task_id: task.task_id.clone(),
        user_id: task.user_id,
        chat_id: task.chat_id,
        user_key: task.user_key.clone(),
        channel: task.channel.clone(),
        external_user_id: task.external_user_id.clone(),
        external_chat_id: task.external_chat_id.clone(),
        execution_policy_stamp,
    }
}

fn finalize_inline_child_success(
    state: &AppState,
    child_task: &crate::ClaimedTask,
    child_payload: &Value,
    result: &Value,
) -> Result<(), String> {
    let persisted = json!({
        "schema_version": 1,
        "source": "inline_child_agent_loop",
        "child_model_result": result,
    });
    crate::repo::update_task_success(
        state,
        &child_task.task_id,
        child_task.claim_attempt,
        &persisted.to_string(),
    )
    .map_err(|err| format!("subagent_inline_child_success_persistence_failed detail={err}"))?;
    crate::repo::child_tasks::record_child_task_terminal_projection(
        state,
        &child_task.task_id,
        child_payload,
    )
    .map_err(|err| format!("subagent_inline_child_terminal_projection_failed detail={err}"))?;
    Ok(())
}

fn finalize_inline_child_failure(
    state: &AppState,
    child_task: &crate::ClaimedTask,
    child_payload: &Value,
    result: &Value,
    error_code: &str,
) {
    let persisted = json!({
        "schema_version": 1,
        "source": "inline_child_agent_loop",
        "child_model_result": result,
    });
    let _ = crate::repo::update_task_failure_with_result(
        state,
        &child_task.task_id,
        child_task.claim_attempt,
        &persisted.to_string(),
        error_code,
    );
    let _ = crate::repo::child_tasks::record_child_task_terminal_projection(
        state,
        &child_task.task_id,
        child_payload,
    );
}

fn parse_child_loop_result(
    raw: &str,
    role: &str,
    context_refs: &Value,
    result_contract: &Value,
) -> Value {
    let parsed = serde_json::from_str::<Value>(raw.trim())
        .ok()
        .filter(Value::is_object)
        .or_else(|| {
            json_object_candidates(raw)
                .into_iter()
                .filter_map(|candidate| serde_json::from_str::<Value>(&candidate).ok())
                .filter(Value::is_object)
                .max_by_key(|candidate| candidate.to_string().len())
        });
    let Some(parsed) = parsed else {
        return child_loop_error_result("subagent_child_json_parse_failed", raw);
    };
    let mut result = if valid_child_model_result(&parsed) {
        let mut result = parsed.clone();
        normalize_child_model_result(&mut result);
        result
    } else if let Some(result) =
        wrap_satisfied_child_result_contract(&parsed, role, context_refs, result_contract)
    {
        result
    } else {
        child_loop_error_result("subagent_child_result_contract_invalid", "")
    };
    if result.get("status").and_then(Value::as_str) == Some("completed") {
        let Some(contract_result) = child_result_contract_projection(&parsed, result_contract)
        else {
            return child_loop_error_result("subagent_child_result_contract_invalid", "");
        };
        if let Some(object) = result.as_object_mut() {
            object.insert("role".to_string(), json!(role));
            object.insert("result".to_string(), contract_result);
        }
    }
    if result.get("status").and_then(Value::as_str) == Some("failed")
        && result.get("role").is_none()
    {
        let mut result = result;
        if let Some(object) = result.as_object_mut() {
            object.insert("role".to_string(), json!(role));
            object.insert(
                "input_evidence_ref_count".to_string(),
                json!(context_refs.as_array().map_or(0, Vec::len)),
            );
        }
        return result;
    }
    result
}

fn child_result_contract_projection(candidate: &Value, result_contract: &Value) -> Option<Value> {
    let required_keys = result_contract
        .get("required_keys")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if required_keys.is_empty() {
        return Some(json!({}));
    }
    let source = candidate
        .get("result")
        .and_then(Value::as_object)
        .or_else(|| candidate.as_object())?;
    let mut projection = serde_json::Map::new();
    for key in required_keys {
        projection.insert(key.to_string(), source.get(key)?.clone());
    }
    Some(Value::Object(projection))
}

fn child_result_evidence_refs(candidate: &Value, context_refs: &Value) -> Vec<Value> {
    let mut refs = candidate
        .get("evidence_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| json!(value))
        .collect::<Vec<_>>();
    for key in ["evidence_ref", "evidence_path"] {
        if let Some(value) = candidate
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            refs.push(json!(value));
        }
    }
    refs.extend(
        context_refs
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| json!(value)),
    );
    refs.dedup();
    refs
}

fn wrap_satisfied_child_result_contract(
    candidate: &Value,
    role: &str,
    context_refs: &Value,
    result_contract: &Value,
) -> Option<Value> {
    let result = child_result_contract_projection(candidate, result_contract)?;
    let evidence_refs = child_result_evidence_refs(candidate, context_refs);
    if result_contract
        .get("require_evidence")
        .and_then(Value::as_bool)
        == Some(true)
        && evidence_refs.is_empty()
    {
        return None;
    }
    let status = candidate
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| matches!(*status, "completed" | "needs_more_evidence" | "failed"))
        .unwrap_or("completed");
    let findings = candidate
        .get("findings")
        .and_then(Value::as_array)
        .filter(|items| items.iter().all(Value::is_object))
        .cloned()
        .unwrap_or_else(|| {
            vec![json!({
                "code": "result_contract_satisfied",
                "summary": "child returned every required result-contract key"
            })]
        });
    let confidence = candidate
        .get("confidence")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .unwrap_or(0.8);
    Some(json!({
        "schema_version": 1,
        "owner_layer": "subagent_model_child",
        "output_format": "machine_json",
        "status": status,
        "role": role,
        "findings": findings,
        "evidence_refs": evidence_refs,
        "confidence": confidence,
        "result": result,
    }))
}

#[cfg(test)]
fn parse_child_model_result(raw: &str) -> Value {
    let parsed = serde_json::from_str::<Value>(raw.trim())
        .ok()
        .filter(Value::is_object)
        .or_else(|| extract_child_result_object(raw));
    let Some(mut value) = parsed else {
        return child_loop_error_result("subagent_child_json_parse_failed", raw);
    };
    if !valid_child_model_result(&value) {
        return child_loop_error_result("subagent_child_result_contract_invalid", "");
    }
    normalize_child_model_result(&mut value);
    value
}

#[cfg(test)]
fn extract_child_result_object(raw: &str) -> Option<Value> {
    json_object_candidates(raw)
        .into_iter()
        .filter_map(|candidate| serde_json::from_str::<Value>(&candidate).ok())
        .filter(valid_child_model_result)
        .max_by_key(child_result_object_score)
}

fn valid_child_model_result(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("schema_version").and_then(Value::as_u64) != Some(1)
        || object.get("owner_layer").and_then(Value::as_str) != Some("subagent_model_child")
        || object.get("output_format").and_then(Value::as_str) != Some("machine_json")
        || !matches!(
            object.get("status").and_then(Value::as_str),
            Some("completed" | "needs_more_evidence" | "failed")
        )
        || !object
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        || !object
            .get("findings")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().all(Value::is_object))
        || !object
            .get("evidence_refs")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().all(Value::is_string))
    {
        return false;
    }
    object
        .get("confidence")
        .and_then(Value::as_f64)
        .is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value))
}

#[cfg(test)]
fn child_result_object_score(value: &Value) -> usize {
    let Some(object) = value.as_object() else {
        return 0;
    };
    let mut score = 0usize;
    if object
        .get("owner_layer")
        .and_then(Value::as_str)
        .is_some_and(|owner| owner == "subagent_model_child")
    {
        score += 4;
    }
    if object
        .get("output_format")
        .and_then(Value::as_str)
        .is_some_and(|format| format == "machine_json")
    {
        score += 3;
    }
    if object.get("status").and_then(Value::as_str).is_some() {
        score += 2;
    }
    if object.get("findings").and_then(Value::as_array).is_some() {
        score += 2;
    }
    if object
        .get("evidence_refs")
        .and_then(Value::as_array)
        .is_some()
    {
        score += 1;
    }
    if object.get("role").and_then(Value::as_str).is_some() {
        score += 1;
    }
    score
}

fn json_object_candidates(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut candidates = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        let mut j = start;
        while j < bytes.len() {
            let c = bytes[j];
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_string = false;
                }
                j += 1;
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        candidates.push(text[start..=j].to_string());
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i = start + 1;
    }
    candidates
}

fn child_loop_error_result(error_code: &str, err: &str) -> Value {
    json!({
        "schema_version": 1,
        "owner_layer": "subagent_model_child",
        "output_format": "machine_json",
        "status": "failed",
        "error_code": error_code,
        "message_key": "clawd.subagent.child_loop_failed",
        "error_excerpt": bounded_error(err),
        "findings": [],
        "evidence_refs": [],
        "confidence": 0.0,
    })
}

fn normalize_child_model_result(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.retain(|key, _| {
        matches!(
            key.as_str(),
            "schema_version"
                | "owner_layer"
                | "output_format"
                | "status"
                | "role"
                | "findings"
                | "finding_refs"
                | "evidence_refs"
                | "artifact_refs"
                | "confidence"
                | "error_code"
                | "message_key"
                | "result"
        )
    });
}

pub(super) fn apply_model_assisted_batch_results(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    results: Vec<(String, bool, Value)>,
) -> bool {
    let Some(observation) = loop_state
        .task_observations
        .iter_mut()
        .rev()
        .find(|observation| {
            observation
                .get("owner_layer")
                .and_then(Value::as_str)
                .is_some_and(|owner| owner == "subagent_runtime")
                && observation
                    .get("global_step")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == global_step)
                && observation
                    .get("step_in_round")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == step_in_round)
        })
    else {
        return false;
    };
    let Some(object) = observation.as_object_mut() else {
        return false;
    };
    let replacements = results
        .into_iter()
        .map(|(child_run_id, required, result)| {
            let status = result
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            let completed = status == "completed";
            let findings = result.get("findings").cloned().unwrap_or_else(|| json!([]));
            let evidence_refs = result
                .get("evidence_refs")
                .cloned()
                .unwrap_or_else(|| json!([]));
            (
                child_run_id.clone(),
                json!({
                    "schema_version": 1,
                    "output_format": "machine_json",
                    "child_run_id": child_run_id,
                    "status": status,
                    "result_status": status,
                    "outcome_code": if completed {
                        "subagent_inline_readonly_completed"
                    } else {
                        "subagent_inline_readonly_failed"
                    },
                    "role": result.get("role"),
                    "required": required,
                    "findings": findings,
                    "finding_count": findings.as_array().map_or(0, Vec::len),
                    "evidence_refs": evidence_refs,
                    "model_result": result,
                    "model_assisted": true,
                    "write_enabled": false,
                    "external_publish_enabled": false,
                    "failure_isolated": !required || completed,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let prior = object
        .get("child_results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let child_results = prior
        .into_iter()
        .map(|item| {
            item.get("child_run_id")
                .and_then(Value::as_str)
                .and_then(|child_run_id| replacements.get(child_run_id))
                .cloned()
                .unwrap_or(item)
        })
        .collect::<Vec<_>>();
    let required_failed_count = child_results
        .iter()
        .filter(|result| {
            result.get("required").and_then(Value::as_bool) == Some(true)
                && result.get("status").and_then(Value::as_str) != Some("completed")
        })
        .count();
    let optional_failed_count = child_results
        .iter()
        .filter(|result| {
            result.get("required").and_then(Value::as_bool) != Some(true)
                && result.get("status").and_then(Value::as_str) != Some("completed")
        })
        .count();
    let completed_count = child_results
        .iter()
        .filter(|result| result.get("status").and_then(Value::as_str) == Some("completed"))
        .count();
    let evidence_refs = child_results
        .iter()
        .flat_map(|result| {
            result
                .get("evidence_refs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    let finding_refs = child_results
        .iter()
        .filter(|result| {
            result
                .get("findings")
                .and_then(Value::as_array)
                .is_some_and(|findings| !findings.is_empty())
        })
        .filter_map(|result| result.get("child_run_id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let status = if required_failed_count > 0 {
        "failed_required_child"
    } else if optional_failed_count > 0 {
        "partial"
    } else {
        "completed"
    };
    object.insert("model_assisted".to_string(), json!(true));
    object.insert("status".to_string(), json!(status));
    object.insert("result_status".to_string(), json!(status));
    object.insert(
        "delegated_terminal_evidence".to_string(),
        json!(required_failed_count == 0),
    );
    object.insert("child_results".to_string(), json!(child_results));
    if let Some(aggregation) = object.get_mut("aggregation").and_then(Value::as_object_mut) {
        aggregation.insert("status".to_string(), json!(status));
        aggregation.insert("completed_count".to_string(), json!(completed_count));
        aggregation.insert(
            "required_failed_count".to_string(),
            json!(required_failed_count),
        );
        aggregation.insert(
            "optional_failed_count".to_string(),
            json!(optional_failed_count),
        );
        aggregation.insert("evidence_refs".to_string(), json!(evidence_refs));
        aggregation.insert("finding_refs".to_string(), json!(finding_refs));
    }
    true
}

pub(super) fn apply_model_assisted_child_result(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    child_result: Value,
) -> bool {
    let Some(observation) = loop_state
        .task_observations
        .iter_mut()
        .rev()
        .find(|observation| {
            observation
                .get("owner_layer")
                .and_then(Value::as_str)
                .is_some_and(|owner| owner == "subagent_runtime")
                && observation
                    .get("global_step")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == global_step)
                && observation
                    .get("step_in_round")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == step_in_round)
        })
    else {
        return false;
    };
    let Some(object) = observation.as_object_mut() else {
        return false;
    };
    let status = child_result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed")
        .to_string();
    object.insert("model_assisted".to_string(), json!(true));
    object.insert(
        "execution_mode".to_string(),
        json!("agent_loop_readonly_child_run"),
    );
    object.insert("action".to_string(), json!("subagent_agent_loop_child"));
    object.insert("child_model_result".to_string(), child_result.clone());
    object.insert("agent_loop_assisted".to_string(), json!(true));
    let (scheduler_status, scheduler_reason_code) = match status.as_str() {
        "completed" => (
            "inline_completed",
            "readonly_subagent_model_execution_completed",
        ),
        "needs_more_evidence" => (
            "waiting_for_evidence",
            "readonly_subagent_model_requested_more_evidence",
        ),
        _ => ("inline_failed", "readonly_subagent_model_execution_failed"),
    };
    object.insert("status".to_string(), json!(status.as_str()));
    object.insert(
        "delegated_terminal_evidence".to_string(),
        json!(status == "completed"),
    );
    if let Some(scheduler) = object.get_mut("scheduler").and_then(Value::as_object_mut) {
        scheduler.insert("status".to_string(), json!(scheduler_status));
        scheduler.insert("reason_code".to_string(), json!(scheduler_reason_code));
    }
    if let Some(merge_contract) = object
        .get_mut("merge_contract")
        .and_then(Value::as_object_mut)
    {
        merge_contract.insert("child_trace_merge_status".to_string(), json!("merged"));
        merge_contract.insert("result_status".to_string(), json!(status.as_str()));
    }
    if let Some(child_request) = object
        .get_mut("child_request")
        .and_then(Value::as_object_mut)
    {
        child_request.insert("state".to_string(), json!(status.as_str()));
    }
    if let Some(child_run_summary) = object
        .get_mut("child_run_summary")
        .and_then(Value::as_object_mut)
    {
        child_run_summary.insert("status".to_string(), json!(status.as_str()));
        child_run_summary.insert("result_status".to_string(), json!(status.as_str()));
        child_run_summary.insert("trace_merge_status".to_string(), json!("merged"));
    }
    if let Some(child_result_object) = object
        .get_mut("child_result")
        .and_then(Value::as_object_mut)
    {
        child_result_object.insert("model_assisted".to_string(), json!(true));
        child_result_object.insert("status".to_string(), json!(status.as_str()));
        child_result_object.insert("result_contract_present".to_string(), json!(true));
        child_result_object.insert("result_status".to_string(), json!(status));
        child_result_object.insert(
            "outcome_code".to_string(),
            json!(match scheduler_status {
                "inline_completed" => "subagent_inline_readonly_completed",
                "waiting_for_evidence" => "subagent_inline_readonly_needs_more_evidence",
                _ => "subagent_inline_readonly_failed",
            }),
        );
        child_result_object.insert(
            "finding_refs".to_string(),
            child_result
                .get("finding_refs")
                .cloned()
                .or_else(|| child_result.get("findings").cloned())
                .unwrap_or_else(|| json!([])),
        );
        child_result_object.insert(
            "evidence_refs".to_string(),
            child_result
                .get("evidence_refs")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
    }
    true
}

fn bounded_error(value: &str) -> String {
    value.chars().take(MAX_CHILD_ERROR_CHARS).collect()
}

#[cfg(test)]
pub(super) fn parse_child_model_result_for_test(raw: &str) -> Value {
    parse_child_model_result(raw)
}

#[cfg(test)]
pub(super) fn parse_child_loop_result_for_test(
    raw: &str,
    role: &str,
    context_refs: &Value,
    result_contract: &Value,
) -> Value {
    parse_child_loop_result(raw, role, context_refs, result_contract)
}
