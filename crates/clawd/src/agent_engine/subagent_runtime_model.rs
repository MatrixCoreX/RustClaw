use serde_json::{json, Value};

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

async fn run_readonly_child_agent_loop(
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
    let spec = ChildTaskSpec {
        parent_task_id: task.task_id.clone(),
        child_task_id: child_ref.clone(),
        role: role.to_string(),
        scope: json!({
            "objective": objective,
            "context_refs": context_refs,
            "allowed_capabilities": allowed_capabilities,
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
    let result = parse_child_loop_result(raw_result, role, &context_refs);
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
            .clamp(1, 12),
        max_tool_calls: budget
            .get("max_tool_calls")
            .and_then(Value::as_u64)
            .unwrap_or(16)
            .clamp(1, 64),
        timeout_ms: timeout_ms.clamp(1_000, 3_600_000),
    }
}

fn child_parent_context(task: &crate::ClaimedTask) -> ChildTaskParentContext {
    let execution_policy_stamp = serde_json::from_str::<Value>(&task.payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .get(crate::task_execution_policy::POLICY_PAYLOAD_FIELD)
                .cloned()
        });
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

fn parse_child_loop_result(raw: &str, role: &str, context_refs: &Value) -> Value {
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
    if is_child_result_object(&parsed) {
        return parse_child_model_result(&parsed.to_string());
    }
    let evidence_refs = context_refs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("ref").and_then(Value::as_str))
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "owner_layer": "subagent_model_child",
        "output_format": "machine_json",
        "status": "completed",
        "role": role,
        "findings": [parsed.clone()],
        "evidence_refs": evidence_refs,
        "confidence": 0.5,
        "result": parsed,
    })
}

fn parse_child_model_result(raw: &str) -> Value {
    let parsed = serde_json::from_str::<Value>(raw.trim())
        .ok()
        .filter(is_child_result_object)
        .or_else(|| extract_child_result_object(raw));
    let mut value = parsed.unwrap_or_else(|| {
        json!({
            "status": "failed",
            "error_code": "subagent_child_json_parse_failed",
            "raw_response_excerpt": bounded_error(raw),
        })
    });
    normalize_child_model_result(&mut value);
    value
}

fn extract_child_result_object(raw: &str) -> Option<Value> {
    json_object_candidates(raw)
        .into_iter()
        .filter_map(|candidate| serde_json::from_str::<Value>(&candidate).ok())
        .filter(is_child_result_object)
        .max_by_key(child_result_object_score)
}

fn is_child_result_object(value: &Value) -> bool {
    child_result_object_score(value) >= 4
}

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
    if !value.is_object() {
        *value = json!({
            "status": "failed",
            "error_code": "subagent_child_non_object_result",
        });
    }
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.entry("schema_version").or_insert_with(|| json!(1));
    object
        .entry("owner_layer")
        .or_insert_with(|| json!("subagent_model_child"));
    object
        .entry("output_format")
        .or_insert_with(|| json!("machine_json"));
    object.entry("status").or_insert_with(|| json!("completed"));
    object.entry("findings").or_insert_with(|| json!([]));
    object.entry("evidence_refs").or_insert_with(|| json!([]));
    object.entry("confidence").or_insert_with(|| json!(0.0));
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
