use serde_json::{json, Value};

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
    (contract.get("output_format").and_then(Value::as_str) == Some("machine_json")
        && contract
            .get("required_keys")
            .and_then(Value::as_array)
            .is_some_and(|keys| !keys.is_empty()))
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
    let required_keys = result_contract
        .get("required_keys")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    if required_keys.is_empty() {
        return None;
    }

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
            let source = candidate
                .get("result")
                .filter(|value| value.is_object())
                .unwrap_or(&candidate)
                .as_object()?;
            let projected = required_keys
                .iter()
                .map(|key| {
                    source
                        .get(*key)
                        .cloned()
                        .map(|value| ((*key).to_string(), value))
                })
                .collect::<Option<serde_json::Map<String, Value>>>()?;
            let value = Value::Object(projected);
            (value.to_string().len() <= MAX_STRUCTURED_RESULT_BYTES).then_some(value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_child_result_requires_every_field_without_parsing_prose() {
        let payload = json!({"task_role":"subagent_child","child_task_contract":{
            "scope":{},"result_contract":{"output_format":"machine_json","required_keys":["name","line"]}
        }});
        for answer in ["name is Panel on line 9", r#"{"name":"Panel"}"#] {
            assert_eq!(
                child_result_contract_failure(&payload, answer).unwrap()["error_code"],
                "child_task_result_contract_invalid"
            );
        }
        assert!(child_result_contract_failure(&payload, r#"{"name":"Panel","line":9}"#).is_none());
        assert!(child_result_contract_failure(
            &json!({"text":"ordinary request"}),
            "natural answer"
        )
        .is_none());
        let oversized = json!({"name":"x".repeat(33*1024),"line":9}).to_string();
        assert!(child_result_contract_failure(&payload, &oversized).is_some());
        let projection = validated_child_structured_result(
            &payload,
            &json!({"text":r#"{"name":"Panel","line":9,"unrequested":"omitted"}"#}),
        )
        .unwrap();
        assert_eq!(projection, json!({"name":"Panel","line":9}));
    }
}
