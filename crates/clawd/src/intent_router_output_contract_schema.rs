use serde_json::Value;

use super::{
    contract_value_token, execution_recipe_value_declares_command_payload,
    list_selector_token::list_selector_machine_token_value,
    looks_like_current_workspace_path_alias, normalize_bool_field_with_default,
    normalize_optional_string_field, normalize_output_delivery_intent_for_schema,
    normalize_output_locator_kind_for_schema, normalize_output_response_shape_for_schema,
    normalize_output_semantic_kind_for_schema, normalize_scalar_count_filter_contract_field,
    normalize_schema_token, output_semantic_kind_requires_fresh_evidence,
    parse_output_semantic_kind, parse_positive_usize_value, scalar_json_value_text,
    IntentOutputContract, OutputLocatorKind, OutputSemanticKind,
};

pub(super) fn apply_raw_output_explicit_locator_repair(
    output_contract: &mut IntentOutputContract,
    route_reason: &str,
    request: &str,
) -> Option<&'static str> {
    if !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !crate::RouteReasonMarkers::new(route_reason)
            .has_machine_marker(OutputSemanticKind::RawCommandOutput.as_str())
        || output_contract.locator_kind != OutputLocatorKind::None
        || !output_contract.locator_hint.trim().is_empty()
        || crate::agent_engine::explicit_command_segment_for_policy(request).is_some()
    {
        return None;
    }
    let locator = crate::intent::locator_extractor::extract_explicit_locator_for_fallback(request)?;
    if !matches!(
        locator.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Url
    ) {
        return None;
    }
    output_contract.locator_kind = locator.locator_kind;
    output_contract.locator_hint = locator.locator_hint;
    Some("raw_output_explicit_locator_contract_repair")
}

pub(super) fn coerce_output_contract_value_for_schema(value: &mut Value) {
    if value.is_object() {
        return;
    }

    let mut contract = serde_json::Map::new();
    if let Some(raw) = value.as_str().map(str::trim).filter(|raw| !raw.is_empty()) {
        let response_shape = normalize_output_response_shape_for_schema(raw);
        if response_shape != "free" {
            contract.insert(
                "response_shape".to_string(),
                Value::String(response_shape.to_string()),
            );
        }
        if let Some(semantic_kind) = command_output_semantic_kind_from_string_contract(raw) {
            contract.insert(
                "contract_marker".to_string(),
                Value::String(semantic_kind.as_str().to_string()),
            );
        }
    }
    *value = Value::Object(contract);
}

fn command_output_semantic_kind_from_string_contract(raw: &str) -> Option<OutputSemanticKind> {
    match normalize_schema_token(raw).as_str() {
        "raw"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "combined_command_output"
        | "command_execution_result"
        | "shell_output"
        | "terminal_output" => Some(OutputSemanticKind::RawCommandOutput),
        "command_output_summary"
        | "command_result_summary"
        | "command_output_synthesis"
        | "command_result_synthesis" => Some(OutputSemanticKind::CommandOutputSummary),
        _ => None,
    }
}

fn normalize_output_contract_aliases(contract: &mut serde_json::Map<String, Value>) {
    let raw_type_token = contract
        .get("type")
        .and_then(scalar_json_value_text)
        .map(|value| normalize_schema_token(&value));
    let file_delivery_type = raw_type_token.as_deref().is_some_and(|token| {
        matches!(
            token,
            "file" | "file_token" | "delivery_file" | "attachment" | "artifact_file"
        )
    });
    if !contract.contains_key("response_shape") {
        for alias in ["shape", "answer_shape", "format", "response_format"] {
            if let Some(value) = contract.get(alias).cloned() {
                contract.insert("response_shape".to_string(), value);
                break;
            }
        }
    }
    if !contract.contains_key("response_shape") && file_delivery_type {
        contract.insert(
            "response_shape".to_string(),
            Value::String("file_token".to_string()),
        );
    }
    if !contract.contains_key("response_shape")
        && contract
            .get("type")
            .and_then(Value::as_str)
            .map(normalize_schema_token)
            .is_some_and(|token| matches!(token.as_str(), "list" | "array"))
    {
        contract.insert(
            "response_shape".to_string(),
            Value::String("strict".to_string()),
        );
    }
    if !contract.contains_key("locator_hint") {
        for alias in [
            "filename",
            "file_name",
            "file_path",
            "path",
            "target_path",
            "locator",
            "locator_value",
        ] {
            if let Some(value) = contract
                .get(alias)
                .and_then(scalar_json_value_text)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                contract.insert("locator_hint".to_string(), Value::String(value));
                break;
            }
        }
    }
    if file_delivery_type {
        if !contract.contains_key("delivery_required") {
            contract.insert("delivery_required".to_string(), Value::Bool(true));
        }
        if !contract.contains_key("delivery_intent") {
            contract.insert(
                "delivery_intent".to_string(),
                Value::String("file_single".to_string()),
            );
        }
        if !contract.contains_key("locator_kind")
            && contract
                .get("locator_hint")
                .and_then(scalar_json_value_text)
                .is_some_and(|hint| !hint.trim().is_empty())
        {
            let hint = contract
                .get("locator_hint")
                .and_then(scalar_json_value_text)
                .unwrap_or_default();
            let kind = if hint.contains('/') || hint.contains('\\') {
                "path"
            } else {
                "filename"
            };
            contract.insert("locator_kind".to_string(), Value::String(kind.to_string()));
        }
    }
    if !contract.contains_key("exact_sentence_count") {
        for alias in ["sentence_count", "sentences", "exact_sentences"] {
            if let Some(value) = contract.get(alias).cloned() {
                contract.insert("exact_sentence_count".to_string(), value);
                break;
            }
        }
    }
}

pub(super) fn normalize_output_contract_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let command_payload_declared =
        execution_recipe_value_declares_command_payload(obj.get("execution_recipe"));
    let Some(value) = obj.get_mut("output_contract") else {
        return;
    };
    let raw_scalar_output_contract_token = value.as_str().map(normalize_schema_token);
    coerce_output_contract_value_for_schema(value);
    let Some(contract) = value.as_object_mut() else {
        return;
    };
    normalize_output_contract_aliases(contract);
    let raw_response_shape_token = contract_value_token(contract, "response_shape");
    let raw_locator_kind_token = contract_value_token(contract, "locator_kind");
    let raw_delivery_required_token = contract_value_token(contract, "delivery_required");
    let raw_contract_marker_token = contract_value_token(contract, "contract_marker");
    let raw_locator_hint_token = contract_value_token(contract, "locator_hint");
    contract.retain(|key, _| {
        matches!(
            key.as_str(),
            "response_shape"
                | "exact_sentence_count"
                | "requires_content_evidence"
                | "delivery_required"
                | "locator_kind"
                | "delivery_intent"
                | "contract_marker"
                | "locator_hint"
                | "scalar_count_filter"
                | "list_selector"
                | "self_extension"
        )
    });
    contract
        .entry("response_shape".to_string())
        .or_insert_with(|| Value::String("free".to_string()));
    let response_shape = contract
        .get("response_shape")
        .and_then(|value| value.as_str())
        .map(normalize_output_response_shape_for_schema)
        .unwrap_or("free");
    contract.insert(
        "response_shape".to_string(),
        Value::String(response_shape.to_string()),
    );
    if let Some(value) = contract.get("exact_sentence_count").cloned() {
        if let Some(count) = parse_positive_usize_value(&value) {
            contract.insert(
                "exact_sentence_count".to_string(),
                Value::Number(serde_json::Number::from(count as u64)),
            );
            if count > 1 && response_shape == "one_sentence" {
                contract.insert(
                    "response_shape".to_string(),
                    Value::String("strict".to_string()),
                );
            }
        } else {
            contract.remove("exact_sentence_count");
        }
    }
    contract
        .entry("requires_content_evidence".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(contract, "requires_content_evidence", false);
    contract
        .entry("delivery_required".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(contract, "delivery_required", false);
    contract
        .entry("locator_kind".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    let locator_kind = contract
        .get("locator_kind")
        .and_then(|value| value.as_str())
        .map(normalize_output_locator_kind_for_schema)
        .unwrap_or("none");
    contract.insert(
        "locator_kind".to_string(),
        Value::String(locator_kind.to_string()),
    );
    contract
        .entry("delivery_intent".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    let delivery_intent = contract
        .get("delivery_intent")
        .and_then(|value| value.as_str())
        .map(normalize_output_delivery_intent_for_schema)
        .unwrap_or("none");
    contract.insert(
        "delivery_intent".to_string(),
        Value::String(delivery_intent.to_string()),
    );
    contract
        .entry("contract_marker".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    let semantic_kind = contract
        .get("contract_marker")
        .and_then(|value| value.as_str())
        .map(normalize_output_semantic_kind_for_schema)
        .unwrap_or("none");
    let mut semantic_kind = if raw_scalar_output_contract_token.as_deref() == Some("raw")
        && !command_payload_declared
    {
        "none".to_string()
    } else {
        semantic_kind.to_string()
    };
    if semantic_kind == OutputSemanticKind::DirectoryEntryGroups.as_str()
        && response_shape == "free"
    {
        semantic_kind = OutputSemanticKind::DirectoryPurposeSummary
            .as_str()
            .to_string();
    }
    let declared_semantic_kind_enum = parse_output_semantic_kind(&semantic_kind);
    if declared_semantic_kind_enum.is_normalizer_schema_capability_bridge() {
        semantic_kind = OutputSemanticKind::None.as_str().to_string();
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
    contract.insert(
        "contract_marker".to_string(),
        Value::String(semantic_kind.clone()),
    );
    let semantic_kind_enum = parse_output_semantic_kind(&semantic_kind);
    if matches!(
        semantic_kind.as_str(),
        kind if kind == OutputSemanticKind::FileNames.as_str()
            || kind == OutputSemanticKind::DirectoryNames.as_str()
            || kind == OutputSemanticKind::DirectoryEntryGroups.as_str()
            || kind == OutputSemanticKind::FilePaths.as_str()
    ) && response_shape == "free"
    {
        contract.insert(
            "response_shape".to_string(),
            Value::String("strict".to_string()),
        );
    }
    if semantic_kind == OutputSemanticKind::FileNames.as_str()
        || semantic_kind == OutputSemanticKind::DirectoryNames.as_str()
        || semantic_kind == OutputSemanticKind::DirectoryEntryGroups.as_str()
        || semantic_kind == OutputSemanticKind::FilePaths.as_str()
        || semantic_kind == OutputSemanticKind::ContentPresenceCheck.as_str()
        || semantic_kind == OutputSemanticKind::GitCommitSubject.as_str()
        || semantic_kind == OutputSemanticKind::GitRepositoryState.as_str()
    {
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
    if output_semantic_kind_requires_fresh_evidence(semantic_kind_enum) {
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
    if semantic_kind == OutputSemanticKind::HiddenEntriesCheck.as_str() {
        if matches!(response_shape, "free" | "one_sentence") {
            contract.insert(
                "response_shape".to_string(),
                Value::String("strict".to_string()),
            );
        }
        contract.insert("delivery_required".to_string(), Value::Bool(false));
    }
    if semantic_kind == OutputSemanticKind::ExistenceWithPath.as_str()
        || semantic_kind == OutputSemanticKind::ExistenceWithPathSummary.as_str()
    {
        if matches!(response_shape, "free" | "one_sentence") {
            contract.insert(
                "response_shape".to_string(),
                Value::String("strict".to_string()),
            );
        }
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(false));
    }
    if semantic_kind == OutputSemanticKind::ExecutionFailedStep.as_str() {
        if matches!(response_shape, "free" | "one_sentence") {
            contract.insert(
                "response_shape".to_string(),
                Value::String("strict".to_string()),
            );
        }
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(false));
    }
    if semantic_kind == OutputSemanticKind::FilesystemMutationResult.as_str() {
        if response_shape == "free" {
            contract.insert(
                "response_shape".to_string(),
                Value::String("one_sentence".to_string()),
            );
        }
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(false));
        contract.insert(
            "delivery_intent".to_string(),
            Value::String("none".to_string()),
        );
    }
    if semantic_kind == OutputSemanticKind::GeneratedFileDelivery.as_str() {
        contract.insert(
            "response_shape".to_string(),
            Value::String("file_token".to_string()),
        );
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(true));
        contract.insert(
            "delivery_intent".to_string(),
            Value::String("file_single".to_string()),
        );
        if locator_kind == "none" {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("current_workspace".to_string()),
            );
        }
    }
    if semantic_kind == OutputSemanticKind::GeneratedFilePathReport.as_str() {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(false));
        contract.insert(
            "delivery_intent".to_string(),
            Value::String("none".to_string()),
        );
        if locator_kind == "none" {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("current_workspace".to_string()),
            );
        }
    }
    contract
        .entry("locator_hint".to_string())
        .or_insert_with(|| Value::String(String::new()));
    normalize_optional_string_field(contract, "locator_hint");
    if locator_kind == "none" {
        contract.insert("locator_hint".to_string(), Value::String(String::new()));
    }
    let current_workspace_path_alias = raw_contract_marker_token == "filesystem_locator"
        && (looks_like_current_workspace_path_alias(&raw_locator_hint_token)
            || looks_like_current_workspace_path_alias(&raw_delivery_required_token)
            || looks_like_current_workspace_path_alias(&raw_locator_kind_token));
    if current_workspace_path_alias {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        contract.insert(
            "contract_marker".to_string(),
            Value::String("scalar_path_only".to_string()),
        );
        contract.insert(
            "locator_kind".to_string(),
            Value::String("current_workspace".to_string()),
        );
        contract.insert(
            "delivery_intent".to_string(),
            Value::String("none".to_string()),
        );
        contract.insert("delivery_required".to_string(), Value::Bool(false));
        contract.insert("requires_content_evidence".to_string(), Value::Bool(false));
        contract.insert("locator_hint".to_string(), Value::String(String::new()));
    } else if raw_response_shape_token == "plain_text"
        && looks_like_current_workspace_path_alias(&raw_locator_hint_token)
        && response_shape == "free"
    {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        contract.insert(
            "contract_marker".to_string(),
            Value::String("scalar_path_only".to_string()),
        );
        contract.insert(
            "locator_kind".to_string(),
            Value::String("current_workspace".to_string()),
        );
        contract.insert("locator_hint".to_string(), Value::String(String::new()));
    }
    normalize_scalar_count_filter_contract_field(contract);
    normalize_list_selector_contract_field(contract);
    let self_extension = contract
        .entry("self_extension".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "mode": "none",
                "trigger": "none",
                "execute_now": false
            })
        });
    if !self_extension.is_object() {
        *self_extension = serde_json::json!({
            "mode": "none",
            "trigger": "none",
            "execute_now": false
        });
    }
    if let Some(self_extension) = self_extension.as_object_mut() {
        self_extension.retain(|key, _| matches!(key.as_str(), "mode" | "trigger" | "execute_now"));
        self_extension
            .entry("mode".to_string())
            .or_insert_with(|| Value::String("none".to_string()));
        self_extension
            .entry("trigger".to_string())
            .or_insert_with(|| Value::String("none".to_string()));
        self_extension
            .entry("execute_now".to_string())
            .or_insert(Value::Bool(false));
    }
}
fn normalize_list_selector_contract_field(contract: &mut serde_json::Map<String, Value>) {
    let Some(value) = contract.get_mut("list_selector") else {
        return;
    };
    if let Some(raw) = value.as_str() {
        *value = list_selector_machine_token_value(raw).unwrap_or(Value::Null);
        return;
    }
    let Some(selector) = value.as_object_mut() else {
        *value = Value::Null;
        return;
    };
    selector.retain(|key, _| {
        matches!(
            key.as_str(),
            "target_kind" | "limit" | "sort_by" | "include_metadata" | "include_hidden"
        )
    });
    if let Some(target_kind) = selector.get("target_kind").cloned() {
        let normalized = target_kind
            .as_str()
            .map(normalize_schema_token)
            .unwrap_or_default();
        let target_kind = match normalized.as_str() {
            "file" => "file",
            "dir" | "directory" | "folder" => "dir",
            "any" | "" => "any",
            _ => "any",
        };
        selector.insert(
            "target_kind".to_string(),
            Value::String(target_kind.to_string()),
        );
    }
    if let Some(sort_by) = selector.get("sort_by").cloned() {
        if sort_by.is_null() {
            selector.remove("sort_by");
        } else {
            let normalized = sort_by
                .as_str()
                .map(normalize_schema_token)
                .unwrap_or_default();
            let sort_by = match normalized.as_str() {
                "name" | "name_desc" | "size_desc" | "size_asc" | "mtime_desc" | "mtime_asc"
                | "" => normalized,
                _ => String::new(),
            };
            selector.insert("sort_by".to_string(), Value::String(sort_by));
        }
    }
    if let Some(limit) = selector.get("limit").cloned() {
        if let Some(limit) = parse_positive_usize_value(&limit) {
            selector.insert(
                "limit".to_string(),
                Value::Number(serde_json::Number::from(limit as u64)),
            );
        } else if !limit.is_null() {
            selector.insert("limit".to_string(), Value::Null);
        }
    }
    if selector
        .get("include_metadata")
        .is_some_and(|value| !value.is_boolean())
    {
        selector.insert("include_metadata".to_string(), Value::Null);
    }
    if selector
        .get("include_hidden")
        .is_some_and(|value| !value.is_boolean())
    {
        selector.insert("include_hidden".to_string(), Value::Null);
    }
}
