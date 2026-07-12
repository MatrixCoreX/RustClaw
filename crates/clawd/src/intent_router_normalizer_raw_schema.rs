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
    promote_session_binding_alias_pair_to_state_patch(obj);
    insert_boundary_envelope_for_schema(obj, req);
    retain_intent_normalizer_top_level_schema_fields(obj);
    let report = contract_repair_report_from_before_after(&before, &value);
    (
        serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string()),
        report,
    )
}

fn promote_session_binding_alias_pair_to_state_patch(obj: &mut serde_json::Map<String, Value>) {
    let binding = obj
        .get("session_binding")
        .and_then(session_binding_alias_pair_for_state_patch)
        .or_else(|| {
            obj.get("boundary_envelope")
                .and_then(|value| value.get("session_binding"))
                .and_then(session_binding_alias_pair_for_state_patch)
        });
    let Some((alias, target)) = binding else {
        return;
    };
    let patch = obj
        .entry("state_patch".to_string())
        .or_insert_with(|| json!({}));
    if patch.is_null() {
        *patch = json!({});
    }
    let Some(patch_obj) = patch.as_object_mut() else {
        return;
    };
    let bindings = patch_obj
        .entry("alias_bindings".to_string())
        .or_insert_with(|| json!([]));
    if bindings.is_null() {
        *bindings = json!([]);
    }
    let Some(items) = bindings.as_array_mut() else {
        return;
    };
    if items.iter().any(|item| {
        item.get("alias")
            .or_else(|| item.get("alias_name"))
            .and_then(Value::as_str)
            .is_some_and(|existing| existing.trim().eq_ignore_ascii_case(&alias))
    }) {
        return;
    }
    items.push(json!({
        "alias": alias,
        "target": target,
    }));
}

fn session_binding_alias_pair_for_state_patch(value: &Value) -> Option<(String, String)> {
    let obj = value.as_object()?;
    if obj.get("alias_resolved").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let alias = ["alias", "alias_name", "alias_key", "surface", "name"]
        .into_iter()
        .find_map(|key| obj.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= 128)?
        .to_string();
    let (target, strong_locator_field) = [
        ("alias_target", true),
        ("locator_hint", true),
        ("locator", true),
        ("path", true),
        ("target", false),
        ("value", false),
    ]
    .into_iter()
    .find_map(|(key, strong_locator_field)| {
        obj.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.chars().count() <= 1024)
            .map(|value| (value.to_string(), strong_locator_field))
    })?;
    if !strong_locator_field && !binding_target_looks_like_locator(&target) {
        return None;
    }
    Some((alias, target))
}

fn binding_target_looks_like_locator(target: &str) -> bool {
    let trimmed = target.trim();
    trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("://")
        || trimmed.starts_with('.')
        || trimmed.starts_with('~')
}

fn insert_boundary_envelope_for_schema(obj: &mut serde_json::Map<String, Value>, req: &str) {
    let model_boundary = obj
        .get("boundary_envelope")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
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
    let mut explicit_locators = boundary_string_array_for_schema(
        model_boundary.get("explicit_locators"),
        BoundaryStringKind::Locator,
    );
    if locator_kind != "none" {
        locator_hint
            .map(|value| Value::String(value.to_string()))
            .into_iter()
            .for_each(|value| push_unique_boundary_value(&mut explicit_locators, value));
    }
    let mut attachment_refs = boundary_string_array_for_schema(
        model_boundary.get("attachment_refs"),
        BoundaryStringKind::Reference,
    );
    if obj
        .get("attachment_processing_required")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        push_unique_boundary_value(
            &mut attachment_refs,
            Value::String("current_request_attachments".to_string()),
        );
    }
    let schedule_intent = obj
        .get("schedule_intent")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| model_boundary.get("schedule_intent").cloned())
        .unwrap_or(Value::Null);
    let request_language_hint = crate::language_policy::request_language_hint(req);
    let model_language_hint = boundary_string_for_schema(
        model_boundary
            .get("language_hint")
            .or_else(|| obj.get("language_hint")),
        BoundaryStringKind::Reference,
    )
    .map(Value::String);
    let language_hint = if matches!(request_language_hint, "config_default" | "mixed") {
        model_language_hint.unwrap_or(Value::Null)
    } else {
        Value::String(request_language_hint.to_string())
    };
    let active_task_reference = obj
        .get("target_task_policy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .or_else(|| {
            boundary_string_for_schema(
                model_boundary.get("active_task_reference"),
                BoundaryStringKind::Reference,
            )
            .map(Value::String)
        })
        .unwrap_or(Value::Null);
    let session_binding = obj
        .get("resume_behavior")
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty() && value != "none")
        .map(Value::String)
        .or_else(|| {
            boundary_string_for_schema(obj.get("session_binding"), BoundaryStringKind::Reference)
                .map(Value::String)
        })
        .or_else(|| {
            boundary_string_for_schema(
                model_boundary.get("session_binding"),
                BoundaryStringKind::Reference,
            )
            .map(Value::String)
        })
        .unwrap_or(Value::Null);
    let safety_budget_hint = boundary_string_for_schema(
        model_boundary.get("safety_budget_hint"),
        BoundaryStringKind::Reference,
    )
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
            "safety_budget_hint": safety_budget_hint,
        }),
    );
}

#[derive(Debug, Clone, Copy)]
enum BoundaryStringKind {
    Locator,
    Reference,
}

fn boundary_string_array_for_schema(value: Option<&Value>, kind: BoundaryStringKind) -> Vec<Value> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        if let Some(text) = boundary_string_for_schema(Some(item), kind) {
            push_unique_boundary_value(&mut out, Value::String(text));
        }
    }
    out
}

fn push_unique_boundary_value(values: &mut Vec<Value>, value: Value) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn boundary_string_for_schema(value: Option<&Value>, kind: BoundaryStringKind) -> Option<String> {
    let value = value?;
    let text = value
        .as_str()
        .or_else(|| boundary_object_reference_for_schema(value, kind))?
        .trim();
    if text.is_empty() {
        return None;
    }
    let max_len = match kind {
        BoundaryStringKind::Locator => 1024,
        BoundaryStringKind::Reference => 128,
    };
    if text.chars().count() > max_len || text.chars().any(|ch| ch.is_control()) {
        return None;
    }
    Some(text.to_string())
}

fn boundary_object_reference_for_schema(value: &Value, kind: BoundaryStringKind) -> Option<&str> {
    if !matches!(kind, BoundaryStringKind::Reference) {
        return None;
    }
    let obj = value.as_object()?;
    if obj.get("alias_resolved").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    [
        "alias_value",
        "resolved_value",
        "target",
        "value",
        "alias_target",
        "locator",
        "path",
    ]
    .into_iter()
    .find_map(|key| obj.get(key).and_then(Value::as_str))
    .or_else(|| {
        obj.get("relevant_aliases")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str)
    })
    .or_else(|| {
        let aliases = obj.get("aliases")?.as_object()?;
        let mut values = aliases
            .values()
            .filter_map(Value::as_str)
            .filter(|text| !text.trim().is_empty());
        let first = values.next()?;
        values.next().is_none().then_some(first)
    })
}
