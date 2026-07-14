use serde_json::{json, Value};

use super::{AppState, LoopState};

const SUBAGENT_MODEL_PROMPT_SOURCE: &str = "subagent_child_loop";
const MAX_CHILD_ERROR_CHARS: usize = 512;

pub(super) async fn maybe_run_model_assisted_subagent(
    state: &AppState,
    task: &crate::ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
) {
    let Some(child_input) = child_model_input(loop_state, global_step, step_in_round, args) else {
        return;
    };
    let prompt = render_child_model_prompt(&child_input);
    let child_result = match crate::llm_gateway::run_with_fallback_with_hints(
        state,
        task,
        &prompt,
        SUBAGENT_MODEL_PROMPT_SOURCE,
        crate::ChatRequestHints {
            temperature: Some(0.0),
            max_tokens: Some(2048),
        },
    )
    .await
    {
        Ok(raw) => parse_child_model_result(&raw),
        Err(err) => provider_error_child_result(&err),
    };
    apply_model_assisted_child_result(loop_state, global_step, step_in_round, child_result);
}

fn child_model_input(
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
    let context_evidence = observation.get("context_evidence")?;
    if !context_evidence
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
        "context_evidence": context_evidence,
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

fn render_child_model_prompt(child_input: &Value) -> String {
    let child_input_json =
        serde_json::to_string_pretty(child_input).unwrap_or_else(|_| child_input.to_string());
    format!(
        "You are a read-only child agent inside RustClaw.\n\
Return exactly one JSON object and then stop. Do not use markdown.\n\
Use only CHILD_INPUT. Do not claim file writes, external publication, network actions, or unseen evidence.\n\
Required top-level fields: schema_version, owner_layer, output_format, status, role, findings, evidence_refs, confidence.\n\
Use owner_layer=\"subagent_model_child\" and output_format=\"machine_json\".\n\
status must be one of completed, needs_more_evidence, failed.\n\
findings must be an array of compact objects grounded in context_evidence items.\n\
evidence_refs must cite only paths or refs present in CHILD_INPUT.context_evidence.items.\n\
If the requested comparison cannot be completed from the supplied evidence, use status=\"needs_more_evidence\" and explain the missing machine evidence in findings.\n\n\
CHILD_INPUT:\n{child_input_json}"
    )
}

fn parse_child_model_result(raw: &str) -> Value {
    let parsed = crate::extract_first_json_value_any(raw)
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .or_else(|| serde_json::from_str::<Value>(raw.trim()).ok());
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

fn provider_error_child_result(err: &str) -> Value {
    json!({
        "schema_version": 1,
        "owner_layer": "subagent_model_child",
        "output_format": "machine_json",
        "status": "failed",
        "error_code": "subagent_child_provider_error",
        "message_key": "clawd.subagent.model_child_failed",
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
        json!("model_assisted_readonly_child_run"),
    );
    object.insert("action".to_string(), json!("subagent_model_child"));
    object.insert("child_model_result".to_string(), child_result.clone());
    if let Some(child_result_object) = object
        .get_mut("child_result")
        .and_then(Value::as_object_mut)
    {
        child_result_object.insert("model_assisted".to_string(), json!(true));
        child_result_object.insert("result_contract_present".to_string(), json!(true));
        child_result_object.insert("result_status".to_string(), json!(status));
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
