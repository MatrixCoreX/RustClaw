use serde_json::{json, Value};

fn default_output_contract() -> Value {
    json!({
        "response_shape": "free",
        "exact_sentence_count": null,
        "requires_content_evidence": false,
        "delivery_required": false,
        "locator_kind": "none",
        "delivery_intent": "none",
        "contract_marker": "none",
        "locator_hint": "",
        "self_extension": {
            "mode": "none",
            "trigger": "none",
            "execute_now": false
        }
    })
}

pub(super) fn normalize_schema_token_for_contract(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .trim_matches('_')
        .to_string()
}

fn locator_hint_is_path_like(hint: &str) -> bool {
    let hint = hint.trim();
    hint.starts_with('/')
        || hint.starts_with("./")
        || hint.starts_with("../")
        || hint.starts_with("~/")
        || hint.contains('/')
        || hint.contains('\\')
}

pub(super) fn normalize_output_contract_locator_kind(
    raw: &str,
    locator_hint: &str,
) -> &'static str {
    match normalize_schema_token_for_contract(raw).as_str() {
        "path" | "file_path" | "directory" | "directory_path" | "dir" => "path",
        "file" | "file_locator" => {
            if locator_hint_is_path_like(locator_hint) {
                "path"
            } else {
                "filename"
            }
        }
        "current_workspace" | "workspace" | "repo" | "repository" => "current_workspace",
        "url" | "uri" | "link" => "url",
        "filename" | "file_name" | "basename" => "filename",
        _ => "none",
    }
}

fn normalize_output_contract_response_shape(raw: &str) -> &'static str {
    match normalize_schema_token_for_contract(raw).as_str() {
        "one_sentence" | "single_sentence" | "sentence" | "short_sentence" => "one_sentence",
        "strict" | "exact" | "exact_text" | "strict_text" | "exact_format" | "one_line"
        | "single_line" | "line_only" | "list" | "array" | "string_list" => "strict",
        "scalar" | "value" | "value_only" | "single_value" | "field_value" => "scalar",
        "file_token" | "delivery_token" => "file_token",
        _ => "free",
    }
}

fn output_contract_semantic_token_requests_scalar_shape(raw: &str) -> bool {
    matches!(
        normalize_schema_token_for_contract(raw).as_str(),
        "scalar"
            | "scalar_value"
            | "scalar_only"
            | "value"
            | "value_only"
            | "single_value"
            | "field_value"
            | "file_field_value"
    )
}

pub(super) fn normalize_output_contract_delivery_intent(raw: &str) -> &'static str {
    match normalize_schema_token_for_contract(raw).as_str() {
        "file_single" | "single_file" | "file" | "deliver_file" | "file_delivery" => "file_single",
        "directory_lookup" | "dir_lookup" | "directory" | "list_directory" => "directory_lookup",
        "directory_batch_files" | "batch_directory_delivery" | "dir_batch" => {
            "directory_batch_files"
        }
        _ => "none",
    }
}

pub(super) fn normalize_output_contract_semantic_kind(raw: &str) -> &'static str {
    match normalize_schema_token_for_contract(raw).as_str() {
        "none" => "none",
        "raw" | "raw_output" | "command_output" | "shell_output" | "terminal_output" => {
            "raw_command_output"
        }
        "raw_command_output" => "raw_command_output",
        "command_output_summary" | "command_result_summary" | "command_output_synthesis" => {
            "command_output_summary"
        }
        "service_status"
        | "service_state"
        | "service_running_status"
        | "process_status"
        | "process_state"
        | "process_running_status"
        | "daemon_status"
        | "daemon_state" => "service_status",
        "hidden_files"
        | "hidden_entries"
        | "hidden_file_check"
        | "hidden_files_check"
        | "hidden_entry_check"
        | "hidden_entries_check" => "hidden_entries_check",
        "file_names"
        | "file_names_only"
        | "file_name_only"
        | "files_listing"
        | "files_list"
        | "names_only"
        | "entry_names"
        | "directory_entry_names"
        | "file_listing"
        | "file_list"
        | "filename_listing"
        | "filename_list"
        | "filename_only"
        | "filenames_list"
        | "filenames_only"
        | "list_filenames"
        | "list_file_names" => "file_names",
        "directory_names"
        | "directory_names_only"
        | "directory_name_only"
        | "dir_names"
        | "dir_names_only"
        | "folder_names"
        | "folder_names_only"
        | "folders_only" => "directory_names",
        "directory_entry_groups"
        | "directory_file_groups"
        | "file_directory_groups"
        | "entry_kind_groups"
        | "entries_by_kind"
        | "grouped_entries"
        | "grouped_entry_names" => "directory_entry_groups",
        "file_paths" | "file_paths_only" | "path_list" | "paths_list" | "file_path_list" => {
            "file_paths"
        }
        "directory_purpose_summary" | "listing_purpose_summary" | "directory_listing_summary" => {
            "directory_purpose_summary"
        }
        "content_excerpt" | "content_excerpt_summary" | "file_excerpt" | "tail_lines" => {
            "content_excerpt_summary"
        }
        "content_excerpt_with_summary"
        | "excerpt_with_summary"
        | "raw_excerpt_with_summary"
        | "bounded_excerpt_with_summary" => "content_excerpt_with_summary",
        "content_presence_check"
        | "content_contains_check"
        | "content_match_check"
        | "identifier_presence_check"
        | "field_presence_check"
        | "text_presence_check" => "content_presence_check",
        "excerpt_kind_judgment" | "content_excerpt_judgment" | "log_vs_checklist" => {
            "excerpt_kind_judgment"
        }
        "recent_artifacts_judgment" | "artifact_style_classification" => {
            "recent_artifacts_judgment"
        }
        "workspace_summary" | "workspace_project_summary" => "workspace_project_summary",
        "scalar_count" | "count" => "scalar_count",
        "quantity_comparison" | "comparison" => "quantity_comparison",
        "execution_failed_step"
        | "failed_step"
        | "failed_command_step"
        | "execution_failure_step" => "execution_failed_step",
        "generated_file_delivery"
        | "new_file_delivery"
        | "created_file_delivery"
        | "write_then_send_file" => "generated_file_delivery",
        "filesystem_mutation_result"
        | "filesystem_mutation"
        | "fs_mutation_result"
        | "file_mutation_result"
        | "path_mutation_result" => "filesystem_mutation_result",
        "scalar_path_only" => "scalar_path_only",
        "existence_with_path" | "exists_with_path" => "existence_with_path",
        "existence_with_path_summary" => "existence_with_path_summary",
        "recent_scalar_equality_check" | "one_line_comparison" | "single_line_comparison" => {
            "recent_scalar_equality_check"
        }
        "git_commit_subject" | "git_commit_title" | "commit_subject" | "commit_title" => {
            "git_commit_subject"
        }
        "structured_keys" => "structured_keys",
        "config_validation" | "structured_config_validation" => "config_validation",
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            "config_mutation"
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            "config_risk_assessment"
        }
        "tool_discovery"
        | "capability_discovery"
        | "capability_inventory"
        | "skill_discovery"
        | "skill_inventory" => "tool_discovery",
        "sqlite_table_listing" => "sqlite_table_listing",
        "sqlite_table_names_only" => "sqlite_table_names_only",
        "sqlite_database_kind_judgment" => "sqlite_database_kind_judgment",
        "sqlite_schema_version" => "sqlite_schema_version",
        "archive_list" => "archive_list",
        "archive_pack" => "archive_pack",
        "archive_unpack" => "archive_unpack",
        _ => "none",
    }
}

pub(super) fn canonicalize_output_contract(value: Value) -> (Value, bool) {
    let Value::Object(mut map) = value else {
        return (default_output_contract(), true);
    };
    let original_len = map.len();
    let allowed_keys = [
        "response_shape",
        "exact_sentence_count",
        "requires_content_evidence",
        "delivery_required",
        "locator_kind",
        "delivery_intent",
        "contract_marker",
        "semantic_kind",
        "locator_hint",
        "self_extension",
    ];
    map.retain(|key, _| allowed_keys.contains(&key.as_str()));
    let mut normalized = map.len() != original_len;
    if !map.contains_key("contract_marker") {
        if let Some(value) = map.get("semantic_kind").cloned() {
            map.insert("contract_marker".to_string(), value);
            normalized = true;
        }
    }
    let defaults = default_output_contract();
    let default_obj = defaults
        .as_object()
        .expect("default output contract is object");
    for key in allowed_keys {
        if !map.contains_key(key) {
            if let Some(default_value) = default_obj.get(key) {
                map.insert(key.to_string(), default_value.clone());
                normalized = true;
            }
        }
    }
    let locator_hint = map
        .get("locator_hint")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if let Some(Value::String(raw)) = map.get("response_shape").cloned() {
        let canonical = normalize_output_contract_response_shape(&raw);
        if canonical != raw {
            map.insert(
                "response_shape".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    if let Some(Value::String(raw)) = map.get("locator_kind").cloned() {
        let canonical = normalize_output_contract_locator_kind(&raw, &locator_hint);
        if canonical != raw {
            map.insert(
                "locator_kind".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    if let Some(Value::String(raw)) = map.get("delivery_intent").cloned() {
        let canonical = normalize_output_contract_delivery_intent(&raw);
        if canonical != raw {
            map.insert(
                "delivery_intent".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
    }
    let mut semantic_token_requests_scalar_shape = false;
    let marker_value = map
        .get("contract_marker")
        .cloned()
        .or_else(|| map.get("semantic_kind").cloned());
    if let Some(Value::String(raw)) = marker_value {
        semantic_token_requests_scalar_shape =
            output_contract_semantic_token_requests_scalar_shape(&raw);
        let canonical = normalize_output_contract_semantic_kind(&raw);
        if canonical != raw {
            map.insert(
                "contract_marker".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        } else if map.get("contract_marker").is_none() {
            map.insert(
                "contract_marker".to_string(),
                Value::String(canonical.to_string()),
            );
            normalized = true;
        }
        if map.contains_key("semantic_kind") {
            map.insert(
                "semantic_kind".to_string(),
                Value::String(canonical.to_string()),
            );
        }
    }
    if semantic_token_requests_scalar_shape
        && map
            .get("response_shape")
            .and_then(Value::as_str)
            .is_some_and(|shape| shape != "scalar" && shape != "file_token")
    {
        map.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        normalized = true;
    }
    let self_extension = map
        .remove("self_extension")
        .unwrap_or_else(|| default_obj["self_extension"].clone());
    let self_extension = match self_extension {
        Value::Object(mut extension) => {
            let original_len = extension.len();
            let allowed_extension_keys = ["mode", "trigger", "execute_now"];
            extension.retain(|key, _| allowed_extension_keys.contains(&key.as_str()));
            normalized |= extension.len() != original_len;
            let default_extension = default_obj["self_extension"]
                .as_object()
                .expect("default self_extension is object");
            for key in allowed_extension_keys {
                if !extension.contains_key(key) {
                    if let Some(default_value) = default_extension.get(key) {
                        extension.insert(key.to_string(), default_value.clone());
                        normalized = true;
                    }
                }
            }
            Value::Object(extension)
        }
        _ => {
            normalized = true;
            default_obj["self_extension"].clone()
        }
    };
    map.insert("self_extension".to_string(), self_extension);
    (Value::Object(map), normalized)
}
