use serde_json::{json, Value};

const SOURCE: &str = "host_child_result_contract";

fn decode(value: &Value) -> Option<Value> {
    if value.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let code = value.get("error_code")?.as_str()?;
    let key = match code {
        "child_result_evidence_invalid" => "child_task.result_evidence_invalid",
        "child_task_result_contract_invalid" => "child_task.result_contract_invalid",
        "child_task_needs_parent_input" => "child_task.needs_parent_input",
        _ => return None,
    };
    if value.get("message_key")?.as_str()? != key {
        return None;
    }
    let retryable = value.get("retryable")?.as_bool()?;
    Some(json!({"schema_version":1,"source":SOURCE,"error_code":code,
        "message_key":key,"retryable":retryable}))
}

pub(super) fn attach(result: &mut Value, status: &str, error_text: Option<&str>) {
    if let Some(object) = result.as_object_mut() {
        object.remove("child_task_failure");
    }
    if status == "succeeded" {
        return;
    }
    let Some(raw) = error_text.filter(|text| text.len() <= 4096) else {
        return;
    };
    // Only exact versioned JSON from the three host child-contract producers.
    // Ordinary error prose and arbitrary skill/child result fields are not decoded.
    let Some(failure) = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| decode(&v))
    else {
        return;
    };
    if let Some(object) = result.as_object_mut() {
        object.insert("child_task_failure".into(), failure);
    }
}

pub(super) fn from_result(result: &Value) -> Option<Value> {
    let failure = result.get("child_task_failure")?;
    (failure.get("source").and_then(Value::as_str) == Some(SOURCE))
        .then(|| decode(failure))
        .flatten()
}

#[cfg(test)]
#[path = "child_task_terminal_failure_tests.rs"]
mod tests;
