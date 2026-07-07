use serde_json::{json, Value};

use super::{
    answer_like_normalizer_payload_text, contract_repair_report_from_before_after,
    normalize_execution_recipe_for_schema, normalize_intent_normalizer_scalar_types_for_schema,
    normalize_intent_normalizer_top_level_for_schema, normalize_output_contract_for_schema,
    normalize_plain_intent_normalizer_text_for_schema, normalize_schema_token,
    parse_top_level_json_object_preserving_meaningful_duplicates,
    retain_intent_normalizer_top_level_schema_fields, ContractRepairReport,
};

#[cfg(test)]
pub(super) fn normalize_intent_normalizer_raw_for_schema(raw: &str, req: &str) -> String {
    normalize_intent_normalizer_raw_for_schema_with_report(raw, req).0
}

pub(super) fn normalize_intent_normalizer_raw_for_schema_with_report(
    raw: &str,
    req: &str,
) -> (String, ContractRepairReport) {
    let parsed_value = parse_top_level_json_object_preserving_meaningful_duplicates(raw)
        .or_else(|| crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw));
    let Some(mut value) = parsed_value else {
        let mut report = ContractRepairReport::default();
        report.add("conservative_none", "raw_parse_failed_safe_chat_schema");
        return (
            normalize_plain_intent_normalizer_text_for_schema(raw, req),
            report,
        );
    };
    let before = value.clone();
    let Some(obj) = value.as_object_mut() else {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| raw.trim());
        let mut report = ContractRepairReport::default();
        report.add("conservative_none", "non_object_output_safe_chat_schema");
        return (
            normalize_plain_intent_normalizer_text_for_schema(text, req),
            report,
        );
    };
    let answer_like_payload = answer_like_normalizer_payload_text(obj);
    match obj.get("resolved_user_intent") {
        Some(Value::String(value)) if value.trim().is_empty() && !req.trim().is_empty() => {
            obj.insert(
                "resolved_user_intent".to_string(),
                Value::String(
                    answer_like_payload
                        .clone()
                        .unwrap_or_else(|| req.trim().to_string()),
                ),
            );
        }
        Some(value) if !value.is_null() && !value.is_string() => {
            let text = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
            obj.insert("resolved_user_intent".to_string(), Value::String(text));
        }
        Some(_) => {}
        None if answer_like_payload.is_some() || !req.trim().is_empty() => {
            obj.insert(
                "resolved_user_intent".to_string(),
                Value::String(
                    answer_like_payload
                        .clone()
                        .unwrap_or_else(|| req.trim().to_string()),
                ),
            );
        }
        None => {}
    }
    normalize_intent_normalizer_top_level_for_schema(obj);
    normalize_intent_normalizer_scalar_types_for_schema(obj);
    normalize_execution_recipe_for_schema(obj, req);
    if let Some(turn_type) = obj.get("turn_type").and_then(|v| v.as_str()) {
        let normalized = normalize_schema_token(turn_type);
        let valid = matches!(
            normalized.as_str(),
            "" | "task_request"
                | "task_append"
                | "task_replace"
                | "task_correct"
                | "task_scope_update"
                | "run_control"
                | "approval_decision"
                | "status_query"
                | "feedback_or_error"
                | "preference_or_memory"
        );
        if !valid {
            obj.insert("turn_type".to_string(), Value::String(String::new()));
        } else {
            obj.insert("turn_type".to_string(), Value::String(normalized));
        }
    }
    if let Some(target_task_policy) = obj.get("target_task_policy").and_then(|v| v.as_str()) {
        let normalized = normalize_schema_token(target_task_policy);
        let valid = matches!(
            normalized.as_str(),
            "" | "reuse_active" | "replace_active" | "pause_and_queue" | "standalone"
        );
        if !valid {
            obj.insert(
                "target_task_policy".to_string(),
                Value::String(String::new()),
            );
        } else {
            obj.insert("target_task_policy".to_string(), Value::String(normalized));
        }
    }
    normalize_output_contract_for_schema(obj);
    insert_boundary_envelope_for_schema(obj, req);
    retain_intent_normalizer_top_level_schema_fields(obj);
    let report = contract_repair_report_from_before_after(&before, &value);
    (
        serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string()),
        report,
    )
}

fn insert_boundary_envelope_for_schema(obj: &mut serde_json::Map<String, Value>, req: &str) {
    let output_contract = obj.get("output_contract").and_then(Value::as_object);
    let locator_kind = output_contract
        .and_then(|contract| contract.get("locator_kind"))
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .unwrap_or_else(|| "none".to_string());
    let locator_hint = output_contract
        .and_then(|contract| contract.get("locator_hint"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let explicit_locators = if locator_kind != "none" {
        locator_hint
            .map(|value| vec![Value::String(value.to_string())])
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let attachment_refs = if obj
        .get("attachment_processing_required")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        vec![Value::String("current_request_attachments".to_string())]
    } else {
        Vec::new()
    };
    let schedule_intent = obj.get("schedule_intent").cloned().unwrap_or(Value::Null);
    let language_hint = {
        let hint = crate::language_policy::request_language_hint(req);
        if hint == "config_default" {
            Value::Null
        } else {
            Value::String(hint.to_string())
        }
    };
    let active_task_reference = obj
        .get("target_task_policy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null);
    let session_binding = obj
        .get("resume_behavior")
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty() && value != "none")
        .map(Value::String)
        .unwrap_or(Value::Null);

    obj.insert(
        "boundary_envelope".to_string(),
        json!({
            "schema_version": crate::intent_router::BOUNDARY_ENVELOPE_SCHEMA_VERSION,
            "raw_chars": req.chars().count(),
            "language_hint": language_hint,
            "schedule_intent": schedule_intent,
            "attachment_refs": attachment_refs,
            "explicit_locators": explicit_locators,
            "active_task_reference": active_task_reference,
            "session_binding": session_binding,
            "safety_budget_hint": Value::Null,
        }),
    );
}
