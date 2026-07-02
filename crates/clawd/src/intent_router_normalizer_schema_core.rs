use serde_json::Value;

use super::{
    execution_recipe_value_declares_command_payload,
    execution_recipe_value_declares_package_detect_manager_capability,
    execution_recipe_value_declares_scalar_runtime_tool_observation,
    execution_recipe_value_declares_service_status_observation,
    execution_recipe_value_declares_structured_read_observation,
    execution_recipe_value_declares_structured_scalar_extraction,
    execution_recipe_value_locator_hint, execution_recipe_value_structured_locator_hint,
    force_output_contract_marker, mark_output_contract_requires_content_evidence,
    normalize_output_contract_for_command_payload,
    normalize_output_contract_for_package_detect_manager_capability,
    normalize_output_contract_for_schema, normalize_output_contract_for_service_status_recipe,
    normalize_output_contract_for_structured_read_recipe, normalize_schema_token,
    normalizer_object_declares_tool_action_payload, output_recipe_value_declares_execution,
    promote_misnested_turn_analysis_from_execution_recipe, request_uses_filename_only_schema_token,
    scalar_json_value_text, scalar_runtime_status_kind_from_execution_recipe,
    scalar_runtime_status_kind_from_output_contract, upsert_runtime_status_query_state_patch,
    OutputSemanticKind,
};

pub(super) fn normalize_plain_intent_normalizer_text_for_schema(raw: &str, req: &str) -> String {
    let text = raw.trim();
    if text.is_empty() {
        return raw.to_string();
    }
    let mut obj = serde_json::Map::new();
    obj.insert(
        "resolved_user_intent".to_string(),
        Value::String(if text.is_empty() { req.trim() } else { text }.to_string()),
    );
    normalize_intent_normalizer_top_level_for_schema(&mut obj);
    normalize_intent_normalizer_scalar_types_for_schema(&mut obj);
    normalize_execution_recipe_for_schema(&mut obj, req);
    normalize_output_contract_for_schema(&mut obj);
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| raw.to_string())
}

pub(super) fn normalize_intent_normalizer_scalar_types_for_schema(
    obj: &mut serde_json::Map<String, Value>,
) {
    normalize_answer_candidate_field(obj);
    normalize_optional_string_field(obj, "clarify_question");
    normalize_optional_string_field(obj, "agent_display_name_hint");
    normalize_optional_string_field(obj, "reason");
    normalize_optional_string_field(obj, "turn_type");
    normalize_optional_string_field(obj, "target_task_policy");
    normalize_confidence_field(obj, "confidence");
}

fn normalize_answer_candidate_field(obj: &mut serde_json::Map<String, Value>) {
    obj.insert("answer_candidate".to_string(), Value::String(String::new()));
}

fn normalize_string_field_with_default(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    default: &str,
) {
    match obj.get(key) {
        Some(Value::String(_)) => {}
        Some(Value::Null) | None => {
            obj.insert(key.to_string(), Value::String(default.to_string()));
        }
        Some(value) => {
            let text = scalar_json_value_text(value).unwrap_or_else(|| default.to_string());
            obj.insert(key.to_string(), Value::String(text));
        }
    }
}

pub(super) fn normalize_optional_string_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    match obj.get(key) {
        Some(Value::String(_)) => {}
        Some(Value::Null) | None => {
            obj.insert(key.to_string(), Value::String(String::new()));
        }
        Some(value) => {
            let text = scalar_json_value_text(value).unwrap_or_else(|| {
                serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
            });
            obj.insert(key.to_string(), Value::String(text));
        }
    }
}

pub(super) fn normalize_bool_field_with_default(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    default: bool,
) {
    let normalized = match obj.get(key) {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::Null) | None => Some(default),
        Some(Value::String(value)) => match normalize_schema_token(value).as_str() {
            "true" | "yes" | "required" => Some(true),
            "false" | "no" | "none" | "final" | "filename_list" | "confirmation" => Some(false),
            _ => Some(default),
        },
        Some(value) => value.as_bool().or(Some(default)),
    };
    if let Some(value) = normalized {
        obj.insert(key.to_string(), Value::Bool(value));
    }
}

fn normalize_confidence_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    let numeric = match obj.get(key) {
        Some(Value::String(confidence)) => {
            let normalized = confidence.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "high" => Some(0.9),
                "medium" => Some(0.6),
                "low" => Some(0.3),
                _ => normalized.parse::<f64>().ok(),
            }
        }
        Some(value) => value.as_f64(),
        None => None,
    };
    if let Some(numeric) = numeric.filter(|value| value.is_finite()) {
        let normalized = if numeric > 1.0 && numeric <= 100.0 {
            numeric / 100.0
        } else {
            numeric
        };
        obj.insert(key.to_string(), Value::from(normalized.clamp(0.0, 1.0)));
    }
}

pub(super) fn normalize_intent_normalizer_top_level_for_schema(
    obj: &mut serde_json::Map<String, Value>,
) {
    obj.remove("mode");
    obj.remove("decision");
    obj.entry("resume_behavior".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    normalize_string_field_with_default(obj, "resume_behavior", "none");
    normalize_resume_behavior_for_schema(obj);
    obj.entry("schedule_kind".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    normalize_string_field_with_default(obj, "schedule_kind", "none");
    normalize_schedule_kind_for_schema(obj);
    normalize_schedule_intent_for_schema(obj);
    obj.entry("wants_file_delivery".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "wants_file_delivery", false);
    obj.entry("should_refresh_long_term_memory".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "should_refresh_long_term_memory", false);
    obj.entry("agent_display_name_hint".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("answer_candidate".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("needs_clarify".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "needs_clarify", false);
    obj.entry("clarify_question".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("reason".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("confidence".to_string())
        .or_insert_with(|| Value::from(0.8));
    obj.entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    obj.entry("execution_recipe".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "kind": "none",
                "profile": "none",
                "target_scope": "none"
            })
        });
    obj.entry("turn_type".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("target_task_policy".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("should_interrupt_active_run".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "should_interrupt_active_run", false);
    obj.entry("state_patch".to_string()).or_insert(Value::Null);
    normalize_state_patch_for_schema(obj);
    obj.entry("attachment_processing_required".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "attachment_processing_required", false);
}

fn normalize_schedule_kind_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let raw = obj
        .get("schedule_kind")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let normalized = normalize_schema_token(raw);
    let canonical = match normalized.as_str() {
        "" | "none" => "none",
        "create" => "create",
        "update" | "pause" | "resume" => normalized.as_str(),
        "delete" => "delete",
        "query" | "list" => normalized.as_str(),
        _ if should_promote_schedule_type_token_to_create(obj, normalized.as_str()) => "create",
        _ => "none",
    };
    obj.insert(
        "schedule_kind".to_string(),
        Value::String(canonical.to_string()),
    );
}

fn should_promote_schedule_type_token_to_create(
    obj: &serde_json::Map<String, Value>,
    normalized_schedule_kind: &str,
) -> bool {
    if is_schedule_type_token(normalized_schedule_kind) {
        return true;
    }
    obj.get("schedule_intent")
        .is_some_and(schedule_intent_value_has_create_signal)
}

fn is_schedule_type_token(token: &str) -> bool {
    matches!(token, "once" | "daily" | "weekly" | "interval" | "cron")
}

fn schedule_intent_value_has_create_signal(value: &Value) -> bool {
    let Value::Object(intent) = value else {
        return false;
    };
    let kind = intent
        .get("kind")
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .unwrap_or_default();
    if kind == "create" {
        return true;
    }
    intent
        .get("schedule")
        .and_then(Value::as_object)
        .and_then(|schedule| schedule.get("type"))
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .is_some_and(|token| is_schedule_type_token(&token))
}

fn normalize_resume_behavior_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let Some(value) = obj.get_mut("resume_behavior") else {
        obj.insert(
            "resume_behavior".to_string(),
            Value::String("none".to_string()),
        );
        return;
    };
    let raw = value.as_str().unwrap_or("none");
    let canonical = match normalize_schema_token(raw).as_str() {
        "resume_execute" | "resume" => "resume_execute",
        "resume_discuss" | "defer" => "resume_discuss",
        _ => "none",
    };
    *value = Value::String(canonical.to_string());
}

fn normalize_schedule_intent_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let schedule_kind_is_none = obj
        .get("schedule_kind")
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .map(|value| value == "none" || value.is_empty())
        .unwrap_or(true);
    let Some(value) = obj.get_mut("schedule_intent") else {
        obj.insert("schedule_intent".to_string(), Value::Null);
        return;
    };
    match value {
        Value::Null => {}
        Value::Object(intent) => {
            if schedule_kind_is_none {
                *value = Value::Null;
                return;
            }
            normalize_schedule_intent_object_for_schema(intent);
            for field in ["schedule", "task"] {
                match intent.get_mut(field) {
                    Some(Value::Object(_)) => {}
                    Some(slot @ Value::String(_)) => {
                        let raw = slot.as_str().unwrap_or_default();
                        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                            *slot = if parsed.is_object() {
                                parsed
                            } else {
                                Value::Object(serde_json::Map::new())
                            };
                        } else {
                            *slot = Value::Object(serde_json::Map::new());
                        }
                    }
                    Some(slot) => {
                        *slot = Value::Object(serde_json::Map::new());
                    }
                    None => {
                        intent.insert(field.to_string(), Value::Object(serde_json::Map::new()));
                    }
                }
            }
        }
        Value::String(raw) => {
            let normalized = normalize_schema_token(raw);
            if normalized.is_empty() || matches!(normalized.as_str(), "none" | "null" | "no") {
                *value = Value::Null;
                return;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                *value = if parsed.is_object() {
                    parsed
                } else {
                    Value::Null
                };
            } else {
                *value = Value::Null;
            }
        }
        _ => {
            *value = Value::Null;
        }
    }
}

fn normalize_schedule_intent_object_for_schema(intent: &mut serde_json::Map<String, Value>) {
    normalize_schedule_intent_string_field(intent, "kind");
    normalize_schedule_intent_string_field(intent, "timezone");
    normalize_schedule_intent_string_field(intent, "target_job_id");
    normalize_schedule_intent_string_field(intent, "raw");
    normalize_schedule_intent_string_field(intent, "reason");
    normalize_schedule_intent_string_field(intent, "clarify_question");
    normalize_bool_field_with_default(intent, "needs_clarify", false);
    intent
        .entry("needs_clarify".to_string())
        .or_insert(Value::Bool(false));
    normalize_confidence_field(intent, "confidence");
    intent
        .entry("confidence".to_string())
        .or_insert_with(|| Value::from(0.0));
    normalize_schedule_intent_schedule_field(intent);
    normalize_schedule_intent_task_field(intent);
}

fn normalize_schedule_intent_string_field(
    intent: &mut serde_json::Map<String, Value>,
    field: &str,
) {
    match intent.get_mut(field) {
        Some(Value::String(_)) => {}
        Some(Value::Null) | None => {
            intent.insert(field.to_string(), Value::String(String::new()));
        }
        Some(slot) => {
            *slot = Value::String(scalar_json_value_text(slot).unwrap_or_default());
        }
    }
}

fn normalize_schedule_intent_schedule_field(intent: &mut serde_json::Map<String, Value>) {
    let schedule = intent
        .entry("schedule".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    match schedule {
        Value::Object(schedule) => {
            for field in ["type", "run_at", "time", "cron"] {
                match schedule.get_mut(field) {
                    Some(Value::String(_)) => {}
                    Some(Value::Null) | None => {
                        schedule.insert(field.to_string(), Value::String(String::new()));
                    }
                    Some(slot) => {
                        *slot = Value::String(scalar_json_value_text(slot).unwrap_or_default());
                    }
                }
            }
            for field in ["weekday", "every_minutes"] {
                let numeric = schedule
                    .get(field)
                    .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
                    .unwrap_or(0);
                schedule.insert(field.to_string(), Value::from(numeric));
            }
        }
        Value::String(raw) => {
            *schedule = serde_json::from_str::<Value>(raw)
                .ok()
                .filter(Value::is_object)
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            if let Value::Object(schedule) = schedule {
                for field in ["type", "run_at", "time", "cron"] {
                    schedule
                        .entry(field.to_string())
                        .or_insert_with(|| Value::String(String::new()));
                }
                for field in ["weekday", "every_minutes"] {
                    schedule.entry(field.to_string()).or_insert(Value::from(0));
                }
            }
        }
        _ => {
            *schedule = Value::Object(serde_json::Map::new());
        }
    }
}

fn normalize_schedule_intent_task_field(intent: &mut serde_json::Map<String, Value>) {
    let message = intent
        .remove("message")
        .and_then(|value| scalar_json_value_text(&value));
    let task = intent
        .entry("task".to_string())
        .or_insert_with(|| schedule_task_from_message(message.as_deref().unwrap_or_default()));
    match task {
        Value::String(raw) => {
            *task = schedule_task_from_message(raw);
        }
        Value::Object(task) => {
            let has_payload = task.get("payload").is_some();
            if !task.get("kind").is_some_and(Value::is_string) {
                task.insert(
                    "kind".to_string(),
                    Value::String(if has_payload || message.is_some() {
                        "ask".to_string()
                    } else {
                        String::new()
                    }),
                );
            }
            match task.get_mut("payload") {
                Some(Value::Object(_)) => {}
                Some(Value::String(raw)) => {
                    let mut payload = serde_json::Map::new();
                    payload.insert("message".to_string(), Value::String(raw.trim().to_string()));
                    task.insert("payload".to_string(), Value::Object(payload));
                }
                Some(Value::Null) | None => {
                    if let Some(message) = message.as_deref().filter(|value| !value.is_empty()) {
                        let mut payload = serde_json::Map::new();
                        payload.insert("message".to_string(), Value::String(message.to_string()));
                        task.insert("payload".to_string(), Value::Object(payload));
                    } else {
                        task.insert("payload".to_string(), Value::Object(serde_json::Map::new()));
                    }
                }
                Some(slot) => {
                    let text = scalar_json_value_text(slot).unwrap_or_default();
                    let mut payload = serde_json::Map::new();
                    payload.insert("message".to_string(), Value::String(text));
                    task.insert("payload".to_string(), Value::Object(payload));
                }
            }
        }
        _ => {
            *task = schedule_task_from_message(message.as_deref().unwrap_or_default());
        }
    }
}

fn schedule_task_from_message(message: &str) -> Value {
    let mut payload = serde_json::Map::new();
    if !message.trim().is_empty() {
        payload.insert(
            "message".to_string(),
            Value::String(message.trim().to_string()),
        );
    }
    let mut task = serde_json::Map::new();
    task.insert("kind".to_string(), Value::String("ask".to_string()));
    task.insert("payload".to_string(), Value::Object(payload));
    Value::Object(task)
}

fn normalize_state_patch_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let Some(value) = obj.get_mut("state_patch") else {
        obj.insert("state_patch".to_string(), Value::Null);
        return;
    };
    match value {
        Value::Null | Value::Object(_) => {}
        Value::String(raw) => {
            let normalized = normalize_schema_token(raw);
            if normalized.is_empty() || matches!(normalized.as_str(), "none" | "null" | "no") {
                *value = Value::Null;
                return;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                *value = if parsed.is_object() {
                    parsed
                } else {
                    Value::Null
                };
            } else {
                *value = Value::Null;
            }
        }
        _ => {
            *value = Value::Null;
        }
    }
}

pub(super) fn normalize_execution_recipe_for_schema(
    obj: &mut serde_json::Map<String, Value>,
    req: &str,
) {
    promote_misnested_turn_analysis_from_execution_recipe(obj);
    if normalizer_object_declares_tool_action_payload(obj) {
        mark_output_contract_requires_content_evidence(obj);
    }
    let execution_recipe_value = obj.get("execution_recipe").cloned();
    let execution_recipe = execution_recipe_value.as_ref();
    if execution_recipe_value_declares_command_payload(execution_recipe) {
        mark_output_contract_requires_content_evidence(obj);
        let locator_hint = execution_recipe_value_locator_hint(execution_recipe);
        normalize_output_contract_for_command_payload(obj, locator_hint.as_deref());
    } else if execution_recipe_value_declares_package_detect_manager_capability(execution_recipe) {
        normalize_output_contract_for_package_detect_manager_capability(obj);
    } else if execution_recipe_value_declares_scalar_runtime_tool_observation(
        execution_recipe,
        obj.get("output_contract"),
    ) {
        mark_output_contract_requires_content_evidence(obj);
        normalize_output_contract_for_command_payload(obj, None);
        force_output_contract_marker(obj, OutputSemanticKind::RawCommandOutput);
        if let Some(kind) = scalar_runtime_status_kind_from_execution_recipe(execution_recipe)
            .or_else(|| scalar_runtime_status_kind_from_output_contract(obj.get("output_contract")))
        {
            upsert_runtime_status_query_state_patch(obj, kind);
        }
    } else if execution_recipe_value_declares_structured_read_observation(execution_recipe) {
        let locator_hint = execution_recipe_value_structured_locator_hint(execution_recipe);
        let scalar_extraction =
            execution_recipe_value_declares_structured_scalar_extraction(execution_recipe);
        normalize_output_contract_for_structured_read_recipe(
            obj,
            locator_hint.as_deref(),
            scalar_extraction,
            request_uses_filename_only_schema_token(req),
        );
    } else if execution_recipe_value_declares_service_status_observation(execution_recipe) {
        normalize_output_contract_for_service_status_recipe(obj);
    } else if output_recipe_value_declares_execution(obj.get("execution_recipe")) {
        mark_output_contract_requires_content_evidence(obj);
    }
    let value = obj
        .entry("execution_recipe".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let Some(recipe) = value.as_object_mut() else {
        return;
    };
    recipe.retain(|key, _| matches!(key.as_str(), "kind" | "profile" | "target_scope"));
    normalize_string_field_with_default(recipe, "kind", "none");
    normalize_string_field_with_default(recipe, "profile", "none");
    normalize_string_field_with_default(recipe, "target_scope", "none");
    if let Some(raw) = recipe.get("kind").and_then(Value::as_str) {
        let kind = crate::execution_recipe::parse_execution_recipe_kind_text(raw);
        recipe.insert("kind".to_string(), Value::String(kind.as_str().to_string()));
    }
    if let Some(raw) = recipe.get("profile").and_then(Value::as_str) {
        let profile = crate::execution_recipe::parse_execution_recipe_profile_text(raw);
        recipe.insert(
            "profile".to_string(),
            Value::String(profile.as_str().to_string()),
        );
    }
    if let Some(raw) = recipe.get("target_scope").and_then(Value::as_str) {
        let target_scope = crate::execution_recipe::parse_execution_recipe_target_scope_text(raw);
        recipe.insert(
            "target_scope".to_string(),
            Value::String(target_scope.as_str().to_string()),
        );
    }
}
