use serde_json::Value;

use super::{
    parse_output_semantic_kind, ActFinalizeStyle, IntentOutputContract, OutputResponseShape,
    OutputSemanticKind,
};

pub(super) fn normalize_schema_token(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .trim_matches('_')
        .to_string()
}

fn is_capability_ref_token(value: &str) -> bool {
    let Some(capability) = value.strip_prefix("capability_ref=") else {
        return false;
    };
    !capability.is_empty()
        && capability.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
        && capability.bytes().any(|byte| byte == b'.')
}

pub(super) fn machine_context_has_capability_ref(machine_context: &str) -> bool {
    machine_context
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')' | '[' | ']'))
        .map(|part| part.trim().to_ascii_lowercase())
        .any(|part| is_capability_ref_token(&part))
}

pub(super) fn normalize_output_response_shape_for_schema(raw: &str) -> &'static str {
    let trimmed = raw.trim();
    if trimmed.contains('{') && trimmed.contains('}') {
        return "strict";
    }
    if trimmed.split_whitespace().nth(1).is_some() {
        return "free";
    }
    match normalize_schema_token(raw).as_str() {
        "one_sentence" | "single_sentence" | "sentence" | "short_sentence" => "one_sentence",
        "strict"
        | "exact"
        | "exact_text"
        | "strict_text"
        | "exact_format"
        | "one_line"
        | "single_line"
        | "line_only"
        | "one_line_string"
        | "single_line_string"
        | "one_line_text"
        | "single_line_text"
        | "one_line_result"
        | "single_line_result"
        | "one_line_comparison"
        | "single_line_comparison"
        | "list"
        | "array"
        | "string_list"
        | "strings_list"
        | "list_of_strings" => "strict",
        "scalar" | "value" | "value_only" | "single_value" | "field_value" => "scalar",
        "file_token" | "file" | "delivery_token" => "file_token",
        // Model-side shape descriptions are not runtime answer contracts.
        _ => "free",
    }
}

pub(super) fn normalize_output_locator_kind_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "path" | "file_path" | "directory" | "directory_path" | "dir" => "path",
        "current_workspace" | "workspace" | "repo" | "repository" => "current_workspace",
        "url" | "uri" | "link" => "url",
        "filename" | "file_name" | "basename" | "file" | "file_locator" => "filename",
        _ => "none",
    }
}

pub(super) fn contract_value_token(contract: &serde_json::Map<String, Value>, key: &str) -> String {
    contract
        .get(key)
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .unwrap_or_default()
}

pub(super) fn looks_like_current_workspace_path_alias(token: &str) -> bool {
    matches!(
        token,
        "current_working_directory"
            | "current_directory"
            | "working_directory"
            | "current_workspace"
            | "workspace_root"
            | "cwd"
            | "pwd"
    )
}

pub(super) fn normalize_output_delivery_intent_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "file_single" | "single_file" | "file" | "deliver_file" | "file_delivery" => "file_single",
        "directory_lookup" | "dir_lookup" | "directory" | "list_directory" => "directory_lookup",
        "directory_batch_files" | "batch_directory_delivery" | "dir_batch" => {
            "directory_batch_files"
        }
        _ => "none",
    }
}

pub(super) fn normalize_output_semantic_kind_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "raw"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "command_execution_result"
        | "shell_output"
        | "terminal_output" => OutputSemanticKind::RawCommandOutput.as_str(),
        "command_output_summary"
        | "command_result_summary"
        | "command_output_synthesis"
        | "command_result_synthesis" => OutputSemanticKind::CommandOutputSummary.as_str(),
        "service_state"
        | "service_running_status"
        | "process_status"
        | "process_state"
        | "process_running_status"
        | "daemon_status"
        | "daemon_state" => OutputSemanticKind::ServiceStatus.as_str(),
        "existence_boolean_with_path" | "boolean_with_path" | "exists_boolean_with_path" => {
            OutputSemanticKind::ExistenceWithPath.as_str()
        }
        "hidden_files"
        | "hidden_entries"
        | "hidden_file_check"
        | "hidden_files_check"
        | "hidden_entry_check"
        | "hidden_entries_check"
        | "hidden_files_example"
        | "hidden_entries_example" => OutputSemanticKind::HiddenEntriesCheck.as_str(),
        "file_names" | "file_names_only" | "file_name_only" | "files_listing" | "files_list"
        | "names_only" | "file_listing" | "file_list" | "filename_listing" | "filename_list"
        | "filename_only" | "filenames_list" | "filenames_only" | "list_filenames"
        | "list_file_names" => OutputSemanticKind::FileNames.as_str(),
        "directory_names"
        | "directory_names_only"
        | "directory_name_only"
        | "dir_names"
        | "dir_names_only"
        | "folder_names"
        | "folder_names_only"
        | "folders_only" => OutputSemanticKind::DirectoryNames.as_str(),
        "directory_entry_groups"
        | "directory_file_groups"
        | "file_directory_groups"
        | "entry_kind_groups"
        | "entry_names"
        | "directory_entry_names"
        | "entries_by_kind"
        | "grouped_entries"
        | "grouped_entry_names" => OutputSemanticKind::DirectoryEntryGroups.as_str(),
        "file_paths"
        | "file_paths_only"
        | "path_list"
        | "paths_list"
        | "file_path_list"
        | "repository_file_paths"
        | "workspace_file_paths" => OutputSemanticKind::FilePaths.as_str(),
        "git_commit_subject"
        | "git_commit_title"
        | "commit_subject"
        | "commit_title"
        | "latest_commit_subject"
        | "latest_commit_title" => OutputSemanticKind::GitCommitSubject.as_str(),
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files"
        | "git_rev_parse" => OutputSemanticKind::GitRepositoryState.as_str(),
        "sqlite_table_names" | "sqlite_table_names_only" | "sqlite_names_only" => {
            OutputSemanticKind::SqliteTableNamesOnly.as_str()
        }
        "one_line_comparison" | "single_line_comparison" => {
            OutputSemanticKind::RecentScalarEqualityCheck.as_str()
        }
        "failed_step" | "failed_command_step" | "execution_failure_step" => {
            OutputSemanticKind::ExecutionFailedStep.as_str()
        }
        "new_file_delivery" | "created_file_delivery" | "write_then_send_file" => {
            OutputSemanticKind::GeneratedFileDelivery.as_str()
        }
        "new_file_path_report"
        | "created_file_path_report"
        | "write_then_report_path"
        | "saved_file_path_report" => OutputSemanticKind::GeneratedFilePathReport.as_str(),
        "file_basename" | "single_file_basename" | "bound_file_basename" => {
            OutputSemanticKind::FileBasename.as_str()
        }
        "value_only" | "file_field_value" | "field_value" => OutputSemanticKind::None.as_str(),
        "document_heading" | "document_title" | "markdown_heading" | "markdown_title"
        | "file_heading" | "file_title" => OutputSemanticKind::DocumentHeading.as_str(),
        normalized => parse_output_semantic_kind(normalized).as_str(),
    }
}

pub(super) fn execution_finalize_style_for_contract(
    contract: &IntentOutputContract,
) -> ActFinalizeStyle {
    if matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::FileToken
    ) || contract.semantic_kind_is(OutputSemanticKind::RawCommandOutput)
    {
        ActFinalizeStyle::Plain
    } else {
        ActFinalizeStyle::ChatWrapped
    }
}
