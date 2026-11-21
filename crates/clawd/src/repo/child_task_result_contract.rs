use serde_json::{json, Value};
#[path = "child_result_schema.rs"]
mod result_schema;

pub(crate) fn child_result_contract_supported(contract: &Value) -> bool {
    if contract
        .get("schema_version")
        .is_some_and(|v| v.as_u64() != Some(1))
    {
        return false;
    }
    if let Some(schema) = contract.get("result_schema") {
        if contract.get("schema_version").and_then(Value::as_u64) != Some(1)
            || !result_schema::supported(schema, 0)
        {
            return false;
        }
    }
    contract.get("evidence_field").is_none_or(|field| {
        field
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 128)
    })
}

pub(crate) fn child_evidence_contract_valid(
    payload: &Value,
    answer: &str,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(contract) = child_machine_result_contract(payload) else {
        return true;
    };
    let Some(field) = contract.get("evidence_field").and_then(Value::as_str) else {
        return true;
    };
    let Some(result) = validated_child_structured_result(payload, &json!({"text":answer})) else {
        return false;
    };
    let Some(refs) = result.get(field).and_then(Value::as_array) else {
        return false;
    };
    !refs.is_empty()
        && refs.len() <= 128
        && refs.iter().all(|reference| {
            reference
                .as_str()
                .and_then(|s| s.strip_prefix("step:"))
                .is_some_and(|id| {
                    journal.step_results.iter().any(|step| {
                        step.step_id == id
                            && step.status == crate::executor::StepExecutionStatus::Ok
                    })
                })
        })
}

pub(crate) fn child_result_contract_for_task(task: &crate::ClaimedTask) -> Option<Value> {
    let payload: Value = serde_json::from_str(&task.payload_json).ok()?;
    child_machine_result_contract(&payload).cloned()
}

pub(crate) fn child_machine_result_valid_for_task(task: &crate::ClaimedTask, answer: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<Value>(&task.payload_json) else {
        return false;
    };
    child_machine_result_contract(&payload).is_some()
        && validated_child_structured_result(&payload, &json!({"text":answer})).is_some()
}

fn child_machine_result_contract(payload: &Value) -> Option<&Value> {
    if !super::is_child_subagent_payload(payload) {
        return None;
    }
    let contract = payload.pointer("/child_task_contract/result_contract")?;
    (contract.get("output_format").and_then(Value::as_str) == Some("machine_json"))
        .then_some(contract)
}

pub(crate) fn child_result_contract_failure(payload: &Value, answer: &str) -> Option<Value> {
    child_machine_result_contract(payload)?;
    validated_child_structured_result(payload, &json!({"text": answer}))
        .is_none()
        .then(|| {
            json!({
                "schema_version": 1,
                "owner_layer": "child_task_result_contract",
                "status": "failed",
                "error_code": "child_task_result_contract_invalid",
                "message_key": "child_task.result_contract_invalid",
                "reason_code": "child_task_result_contract_invalid",
                "status_code": "child_task_result_contract_invalid",
                "retryable": false,
                "side_effect_applied": false,
                "recovery_action": "parent_review_child_result",
            })
        })
}

pub(crate) fn validated_child_structured_result(
    payload: &Value,
    result_json: &Value,
) -> Option<Value> {
    const MAX_STRUCTURED_RESULT_BYTES: usize = 32 * 1024;

    let result_contract = payload.pointer("/child_task_contract/result_contract")?;
    if result_contract.get("output_format").and_then(Value::as_str) != Some("machine_json") {
        return None;
    }
    if !child_result_contract_supported(result_contract) {
        return None;
    }
    let required_keys = match result_contract.get("required_keys") {
        None => Vec::new(),
        Some(keys) => keys
            .as_array()?
            .iter()
            .map(|key| key.as_str().map(str::trim).filter(|key| !key.is_empty()))
            .collect::<Option<Vec<_>>>()?,
    };
    let complete_fields_required = result_contract
        .get("allow_partial")
        .and_then(Value::as_bool)
        == Some(false);

    let candidate_strings = [
        result_json.get("text").and_then(Value::as_str),
        result_json
            .pointer("/final_result_json/text")
            .and_then(Value::as_str),
        result_json
            .pointer("/task_journal/summary/final_answer")
            .and_then(Value::as_str),
    ];
    let direct_candidates = [
        result_json.get("structured_result"),
        result_json.pointer("/child_model_result/result"),
        result_json.pointer("/final_result_json/structured_result"),
    ];

    direct_candidates
        .into_iter()
        .flatten()
        .cloned()
        .chain(
            candidate_strings
                .into_iter()
                .flatten()
                .filter_map(|candidate| {
                    serde_json::from_str::<Value>(candidate.trim())
                        .ok()
                        .or_else(|| {
                            crate::extract_first_json_value_any(candidate)
                                .and_then(|json| serde_json::from_str::<Value>(&json).ok())
                        })
                }),
        )
        .find_map(|candidate| {
            if candidate.to_string().len() > MAX_STRUCTURED_RESULT_BYTES {
                return None;
            }
            let source = candidate.as_object()?;
            if result_contract
                .get("result_schema")
                .is_some_and(|schema| !result_schema::matches(schema, &candidate))
            {
                return None;
            }
            let projected = if required_keys.is_empty() {
                source.clone()
            } else {
                required_keys
                    .iter()
                    .map(|key| {
                        source
                            .get(*key)
                            .filter(|value| !complete_fields_required || !value.is_null())
                            .cloned()
                            .map(|value| ((*key).to_string(), value))
                    })
                    .collect::<Option<serde_json::Map<String, Value>>>()?
            };
            let value = Value::Object(projected);
            (value.to_string().len() <= MAX_STRUCTURED_RESULT_BYTES).then_some(value)
        })
}

#[cfg(test)]
#[path = "child_task_result_contract_tests.rs"]
mod tests;
