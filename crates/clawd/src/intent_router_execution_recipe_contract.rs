use serde_json::Value;

use super::{
    coerce_output_contract_value_for_schema, is_meaningful_state_patch,
    normalize_output_locator_kind_for_schema, parse_output_semantic_kind, scalar_json_value_text,
    OutputSemanticKind,
};

pub(super) fn mark_output_contract_requires_content_evidence(
    obj: &mut serde_json::Map<String, Value>,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    if let Some(contract) = value.as_object_mut() {
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
}

pub(super) fn force_output_contract_semantic_kind(
    obj: &mut serde_json::Map<String, Value>,
    semantic_kind: OutputSemanticKind,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    if let Some(contract) = value.as_object_mut() {
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(semantic_kind.as_str().to_string()),
        );
    }
}

fn is_machine_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'=' | b':')
}

fn machine_context_has_exact_token(context: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let bytes = context.as_bytes();
    let mut offset = 0;
    while let Some(found) = context[offset..].find(expected) {
        let start = offset + found;
        let end = start + expected.len();
        let left_ok = start == 0 || !is_machine_token_byte(bytes[start - 1]);
        let right_ok = end == bytes.len() || !is_machine_token_byte(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        offset = end;
    }
    false
}

fn append_resolved_intent_machine_token(obj: &mut serde_json::Map<String, Value>, token: &str) {
    let current = obj
        .get("resolved_user_intent")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    if machine_context_has_exact_token(&current, token) {
        return;
    }
    let next = if current.trim().is_empty() {
        token.to_string()
    } else {
        format!("{} {}", current.trim(), token)
    };
    obj.insert("resolved_user_intent".to_string(), Value::String(next));
}

pub(super) fn normalize_output_contract_for_structured_read_recipe(
    obj: &mut serde_json::Map<String, Value>,
    locator_hint_from_recipe: Option<&str>,
    scalar_extraction: bool,
    request_declares_filename_only_schema_token: bool,
) {
    let recipe_declares_filename_only_output =
        execution_recipe_declares_filename_only_output(obj.get("execution_recipe"));
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    contract.insert("delivery_required".to_string(), Value::Bool(false));
    contract.insert(
        "delivery_intent".to_string(),
        Value::String("none".to_string()),
    );
    let model_only_filename_semantic = !request_declares_filename_only_schema_token
        && (recipe_declares_filename_only_output
            || contract
                .get("semantic_kind")
                .and_then(scalar_json_value_text)
                .is_some_and(|value| {
                    parse_output_semantic_kind(&value) == OutputSemanticKind::FileNames
                }));

    if scalar_extraction || model_only_filename_semantic {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
    }
    if scalar_extraction || model_only_filename_semantic {
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(OutputSemanticKind::None.as_str().to_string()),
        );
    }
    if request_declares_filename_only_schema_token && recipe_declares_filename_only_output {
        contract.insert(
            "response_shape".to_string(),
            Value::String("strict".to_string()),
        );
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(OutputSemanticKind::FileNames.as_str().to_string()),
        );
    }
    if let Some(hint) = locator_hint_from_recipe
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
    {
        contract.insert(
            "locator_kind".to_string(),
            Value::String("path".to_string()),
        );
        contract.insert("locator_hint".to_string(), Value::String(hint.to_string()));
    }
}

fn execution_recipe_declares_filename_only_output(recipe: Option<&Value>) -> bool {
    recipe
        .and_then(Value::as_object)
        .and_then(|recipe| recipe.get("output"))
        .and_then(scalar_json_value_text)
        .is_some_and(|value| {
            matches!(
                parse_output_semantic_kind(&value),
                OutputSemanticKind::FileNames
            )
        })
}

pub(super) fn normalize_output_contract_for_package_detect_manager_capability(
    obj: &mut serde_json::Map<String, Value>,
) {
    append_resolved_intent_machine_token(obj, PACKAGE_DETECT_MANAGER_CAPABILITY_REF);
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    contract.insert("delivery_required".to_string(), Value::Bool(false));
    contract.insert(
        "delivery_intent".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert(
        "locator_kind".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert("locator_hint".to_string(), Value::String(String::new()));
    contract.insert(
        "semantic_kind".to_string(),
        Value::String(OutputSemanticKind::None.as_str().to_string()),
    );
}

const PACKAGE_DETECT_MANAGER_CAPABILITY_REF: &str =
    concat!("capability_ref=", "package", ".", "detect_manager");

pub(super) fn normalize_output_contract_for_service_status_recipe(
    obj: &mut serde_json::Map<String, Value>,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    contract.insert("delivery_required".to_string(), Value::Bool(false));
    contract.insert(
        "delivery_intent".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert(
        "locator_kind".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert("locator_hint".to_string(), Value::String(String::new()));
    contract.insert(
        "semantic_kind".to_string(),
        Value::String(OutputSemanticKind::ServiceStatus.as_str().to_string()),
    );
}

pub(super) fn normalize_output_contract_for_command_payload(
    obj: &mut serde_json::Map<String, Value>,
    locator_hint_from_recipe: Option<&str>,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    let locator_hint = contract
        .get("locator_hint")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|value| normalize_output_locator_kind_for_schema(&value))
        .unwrap_or("none");
    if locator_hint.trim().is_empty() {
        if let Some(hint) = locator_hint_from_recipe
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
        {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("path".to_string()),
            );
            contract.insert("locator_hint".to_string(), Value::String(hint.to_string()));
        } else if locator_kind != "none" {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("none".to_string()),
            );
            contract.insert("locator_hint".to_string(), Value::String(String::new()));
        }
    } else if locator_kind == "none" {
        contract.insert(
            "locator_kind".to_string(),
            Value::String("path".to_string()),
        );
    }

    let semantic_kind = contract
        .get("semantic_kind")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    if matches!(
        parse_output_semantic_kind(&semantic_kind),
        OutputSemanticKind::None
    ) {
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(OutputSemanticKind::RawCommandOutput.as_str().to_string()),
        );
    }
    contract.insert("delivery_required".to_string(), Value::Bool(false));
}

pub(super) fn promote_misnested_turn_analysis_from_execution_recipe(
    obj: &mut serde_json::Map<String, Value>,
) {
    let Some(recipe) = obj.get("execution_recipe").and_then(Value::as_object) else {
        return;
    };
    let misplaced_turn_type = recipe
        .get("turn_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let misplaced_target_policy = recipe
        .get("target_task_policy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let misplaced_interrupt = recipe
        .get("should_interrupt_active_run")
        .and_then(Value::as_bool)
        .filter(|value| *value);
    let misplaced_attachment = recipe
        .get("attachment_processing_required")
        .and_then(Value::as_bool)
        .filter(|value| *value);
    let misplaced_state_patch = recipe
        .get("state_patch")
        .filter(|value| is_meaningful_state_patch(value))
        .cloned();

    if let Some(turn_type) = misplaced_turn_type {
        if obj
            .get("turn_type")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            obj.insert("turn_type".to_string(), Value::String(turn_type));
        }
    }
    if let Some(target_policy) = misplaced_target_policy {
        if obj
            .get("target_task_policy")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            obj.insert(
                "target_task_policy".to_string(),
                Value::String(target_policy),
            );
        }
    }
    if misplaced_interrupt.is_some()
        && !obj
            .get("should_interrupt_active_run")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        obj.insert("should_interrupt_active_run".to_string(), Value::Bool(true));
    }
    if misplaced_attachment.is_some()
        && !obj
            .get("attachment_processing_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        obj.insert(
            "attachment_processing_required".to_string(),
            Value::Bool(true),
        );
    }
    if let Some(state_patch) = misplaced_state_patch {
        if obj
            .get("state_patch")
            .is_none_or(|value| !is_meaningful_state_patch(value))
        {
            obj.insert("state_patch".to_string(), state_patch);
        }
    }
}
