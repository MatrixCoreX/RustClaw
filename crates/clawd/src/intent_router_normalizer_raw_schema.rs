use serde_json::Value;

use super::{
    answer_candidate_value_text, answer_like_normalizer_payload_text,
    contract_repair_report_from_before_after, normalize_decision_from_executable_output_contract,
    normalize_execution_recipe_for_schema, normalize_intent_normalizer_scalar_types_for_schema,
    normalize_intent_normalizer_top_level_for_schema, normalize_output_contract_for_schema,
    normalize_plain_intent_normalizer_text_for_schema, normalize_schema_token,
    parse_top_level_json_object_preserving_meaningful_duplicates,
    scalar_output_contract_answer_candidate, ContractRepairReport,
};

pub(super) fn merge_answer_candidate_into_resolved_intent(
    resolved: String,
    answer_candidate: &str,
) -> String {
    let answer = answer_candidate.trim();
    if answer.is_empty() || resolved.contains(answer) {
        return resolved;
    }
    if resolved.trim().is_empty() {
        answer.to_string()
    } else {
        format!("{}\nanswer_candidate: {}", resolved.trim(), answer)
    }
}

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
    let explicit_answer_candidate = obj
        .get("answer_candidate")
        .and_then(answer_candidate_value_text)
        .or_else(|| scalar_output_contract_answer_candidate(obj));
    if let Some(candidate) = explicit_answer_candidate {
        let should_insert = obj
            .get("answer_candidate")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty());
        if should_insert {
            obj.insert("answer_candidate".to_string(), Value::String(candidate));
        }
    }
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
    normalize_decision_from_executable_output_contract(obj);
    let report = contract_repair_report_from_before_after(&before, &value);
    (
        serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string()),
        report,
    )
}
