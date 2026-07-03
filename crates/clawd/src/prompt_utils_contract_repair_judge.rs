use serde_json::{json, Value};

use super::{
    canonicalize_output_contract, normalize_output_contract_delivery_intent,
    normalize_output_contract_locator_kind, normalize_output_contract_semantic_kind,
    normalize_schema_token_for_contract,
};

fn canonicalize_contract_repair_judge_execution_recipe(value: Value) -> (Value, bool) {
    let Value::Object(mut recipe) = value else {
        return (
            json!({
                "kind": "none",
                "profile": "none",
                "target_scope": "unknown"
            }),
            true,
        );
    };
    let original_len = recipe.len();
    let allowed_keys = ["kind", "profile", "target_scope"];
    recipe.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = recipe.len() != original_len;

    for (key, default) in [
        ("kind", "none"),
        ("profile", "none"),
        ("target_scope", "unknown"),
    ] {
        if !recipe.contains_key(key) {
            recipe.insert(key.to_string(), Value::String(default.to_string()));
            normalized = true;
        }
        if !recipe.get(key).is_some_and(Value::is_string) {
            recipe.insert(key.to_string(), Value::String(default.to_string()));
            normalized = true;
        }
    }

    if let Some(raw) = recipe.get("kind").and_then(Value::as_str) {
        let canonical = crate::execution_recipe::parse_execution_recipe_kind_text(raw).as_str();
        if canonical != raw {
            recipe.insert("kind".to_string(), Value::String(canonical.to_string()));
            normalized = true;
        }
    }
    if let Some(raw) = recipe.get("profile").and_then(Value::as_str) {
        let canonical = crate::execution_recipe::parse_execution_recipe_profile_text(raw).as_str();
        if canonical != raw {
            recipe.insert("profile".to_string(), Value::String(canonical.to_string()));
            normalized = true;
        }
    }
    if let Some(raw) = recipe.get("target_scope").and_then(Value::as_str) {
        let canonical =
            crate::execution_recipe::parse_execution_recipe_target_scope_text(raw).as_str();
        if canonical != raw {
            recipe.insert(
                "target_scope".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }

    (Value::Object(recipe), normalized)
}

fn canonicalize_contract_repair_judge_turn_type(value: Option<Value>) -> (Value, bool) {
    let Some(Value::String(raw)) = value else {
        return (Value::String(String::new()), true);
    };
    let normalized = normalize_schema_token_for_contract(&raw);
    let canonical = match normalized.as_str() {
        "" | "none" | "null" => "",
        "task_request" => "task_request",
        "task_append" => "task_append",
        "task_replace" => "task_replace",
        "task_correct" => "task_correct",
        "task_scope_update" => "task_scope_update",
        "run_control" => "run_control",
        "approval_decision" => "approval_decision",
        "status_query" => "status_query",
        "feedback_or_error" => "feedback_or_error",
        "preference_or_memory" => "preference_or_memory",
        _ => "",
    };
    (
        Value::String(canonical.to_string()),
        canonical != raw.trim(),
    )
}

fn canonicalize_contract_repair_judge_target_task_policy(value: Option<Value>) -> (Value, bool) {
    let Some(Value::String(raw)) = value else {
        return (Value::String(String::new()), true);
    };
    let normalized = normalize_schema_token_for_contract(&raw);
    let canonical = match normalized.as_str() {
        "" | "none" | "null" => "",
        "reuse_active" => "reuse_active",
        "replace_active" => "replace_active",
        "pause_and_queue" => "pause_and_queue",
        "standalone" => "standalone",
        _ => "",
    };
    (
        Value::String(canonical.to_string()),
        canonical != raw.trim(),
    )
}

fn infer_contract_repair_judge_apply(map: &serde_json::Map<String, Value>) -> bool {
    if map.get("needs_clarify").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    let Some(contract) = map.get("output_contract").and_then(Value::as_object) else {
        return false;
    };
    contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        == Some(true)
        || contract.get("delivery_required").and_then(Value::as_bool) == Some(true)
        || contract
            .get("contract_marker")
            .and_then(Value::as_str)
            .is_some_and(|raw| normalize_output_contract_semantic_kind(raw) != "none")
        || contract
            .get("locator_kind")
            .and_then(Value::as_str)
            .is_some_and(|raw| normalize_output_contract_locator_kind(raw, "") != "none")
        || contract
            .get("delivery_intent")
            .and_then(Value::as_str)
            .is_some_and(|raw| normalize_output_contract_delivery_intent(raw) != "none")
}

fn canonicalize_contract_repair_judge_confidence(value: Option<Value>) -> (Value, bool) {
    match value {
        Some(Value::Number(number)) => (Value::Number(number), false),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if let Ok(parsed) = trimmed.parse::<f64>() {
                let clamped = parsed.clamp(0.0, 1.0);
                return (json!(clamped), true);
            }
            let canonical = match normalize_schema_token_for_contract(trimmed).as_str() {
                "very_high" | "high" => 0.9,
                "medium" | "moderate" => 0.7,
                "low" => 0.4,
                _ => 0.0,
            };
            (json!(canonical), true)
        }
        Some(_) | None => (json!(0.0), true),
    }
}

pub(super) fn canonicalize_contract_repair_judge_object(
    mut map: serde_json::Map<String, Value>,
) -> (Value, bool) {
    let original_len = map.len();
    let allowed_keys = [
        "apply",
        "reason",
        "repair_target",
        "confidence",
        "decision",
        "needs_clarify",
        "clarify_question",
        "resolved_user_intent",
        "output_contract",
        "execution_recipe",
        "turn_type",
        "target_task_policy",
        "state_patch",
    ];
    map.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = map.len() != original_len;

    let (confidence, confidence_normalized) =
        canonicalize_contract_repair_judge_confidence(map.remove("confidence"));
    normalized |= confidence_normalized;
    map.insert("confidence".to_string(), confidence);
    if map
        .insert("decision".to_string(), Value::String(String::new()))
        .is_some()
    {
        normalized = true;
    }

    if let Some(output_contract) = map.remove("output_contract") {
        let (output_contract, contract_normalized) = canonicalize_output_contract(output_contract);
        normalized |= contract_normalized;
        map.insert("output_contract".to_string(), output_contract);
    }
    if let Some(execution_recipe) = map.remove("execution_recipe") {
        let (execution_recipe, recipe_normalized) =
            canonicalize_contract_repair_judge_execution_recipe(execution_recipe);
        normalized |= recipe_normalized;
        map.insert("execution_recipe".to_string(), execution_recipe);
    }
    let (turn_type, turn_type_normalized) =
        canonicalize_contract_repair_judge_turn_type(map.remove("turn_type"));
    normalized |= turn_type_normalized;
    map.insert("turn_type".to_string(), turn_type);
    let (target_task_policy, target_policy_normalized) =
        canonicalize_contract_repair_judge_target_task_policy(map.remove("target_task_policy"));
    normalized |= target_policy_normalized;
    map.insert("target_task_policy".to_string(), target_task_policy);
    if !map.contains_key("apply") {
        let inferred_apply = infer_contract_repair_judge_apply(&map);
        map.insert("apply".to_string(), Value::Bool(inferred_apply));
        normalized = true;
    }

    (Value::Object(map), normalized)
}
