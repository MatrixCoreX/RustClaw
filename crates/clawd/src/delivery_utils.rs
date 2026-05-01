use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{AppState, IntentOutputContract, OutputResponseShape, OutputSemanticKind};

// Facade for delivery interception. Locator parsing, directory lookup, and
// file-resolution flows live in sibling submodules.
mod directory_lookup;
mod file_delivery;
mod locator;
mod message_media;
mod output_contract;
mod path_helpers;
mod types;

use self::directory_lookup::try_handle_directory_lookup_request;
use self::file_delivery::enforce_file_delivery_locator_contract;
pub(crate) use self::file_delivery::scan_filename_matches_with_limit;
pub(crate) use self::locator::extract_filename_candidates;
pub(crate) use self::message_media::{
    collect_recent_image_candidates, extract_file_path_from_delivery_token,
    normalize_delivery_message, trim_path_token,
};
pub(super) use self::output_contract::response_has_same_file_token;
use self::output_contract::{enforce_output_contract, looks_like_delivery_locator_literal};
pub(crate) use self::path_helpers::{
    dedup_and_sort_paths, resolve_existing_dir_under_root, resolve_existing_file_under_root,
    resolve_existing_path_under_root_case_insensitive,
};
#[cfg(test)]
use self::types::localize_delivery_message_for_request;
pub(crate) use self::types::FilenameScanResult;
use self::types::{
    BatchDirectoryDeliveryResolution, CurrentLevelDeliveryEntries,
    CurrentLevelDeliveryEntriesResult, DeliveryMessageKind, DirectoryEntriesListResult,
    DirectoryFileLookupResult, DirectoryLookupInput, DirectoryLookupResolution,
    FileDeliveryLocatorInput, FileDeliveryTargetResolution,
};

pub(crate) fn extract_delivery_file_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some((kind, payload)) = crate::finalize::parse_delivery_token(line) {
            let payload = payload.trim();
            if payload.is_empty() {
                continue;
            }
            out.push(format!("{}{}", kind.canonical_prefix(), payload));
        }
    }
    out
}

pub(crate) fn intercept_response_text_for_delivery(text: &str) -> String {
    text.trim().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectoryLocatorExecutionResolution {
    Resolved(PathBuf),
    MultipleCandidates(Vec<PathBuf>),
    NotFound,
}

pub(crate) fn resolve_directory_locator_for_execution(
    raw_hint: &str,
    default_locator_search_dir: &Path,
    max_depth: usize,
    max_scan_entries: usize,
) -> Option<DirectoryLocatorExecutionResolution> {
    let request = locator::directory_lookup_input_from_hint(raw_hint)?;
    match directory_lookup::resolve_directory_target(
        request,
        Path::new("/"),
        default_locator_search_dir,
        max_depth,
        max_scan_entries,
    ) {
        DirectoryLookupResolution::Resolved(path) => {
            Some(DirectoryLocatorExecutionResolution::Resolved(path))
        }
        DirectoryLookupResolution::MultipleCandidates(paths) => Some(
            DirectoryLocatorExecutionResolution::MultipleCandidates(paths),
        ),
        DirectoryLookupResolution::UserMessage(_) => {
            Some(DirectoryLocatorExecutionResolution::NotFound)
        }
    }
}

pub(crate) fn intercept_response_payload_for_delivery(
    state: &AppState,
    user_request: &str,
    wants_file_delivery: bool,
    output_contract: &IntentOutputContract,
    text: String,
    messages: Vec<String>,
) -> (String, Vec<String>) {
    let mut seen = HashSet::new();
    let mut normalized_messages = messages
        .into_iter()
        .filter_map(|msg| normalize_delivery_message(state, &msg))
        .filter(|msg| !msg.is_empty())
        .filter(|msg| seen.insert(msg.clone()))
        .collect::<Vec<_>>();
    let mut normalized_text = normalize_delivery_message(state, &text)
        .or_else(|| normalized_messages.first().cloned())
        .unwrap_or_default();
    let file_delivery_contract = wants_file_delivery
        || output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        );
    let directory_lookup_candidate = normalized_text.trim();
    let may_replace_with_directory_lookup = directory_lookup_candidate.is_empty()
        || (!matches!(output_contract.semantic_kind, OutputSemanticKind::FileNames)
            && looks_like_delivery_locator_literal(
                directory_lookup_candidate,
                &output_contract.locator_hint,
            ));
    if may_replace_with_directory_lookup {
        if let Some(directory_lookup_text) = try_handle_directory_lookup_request(
            state,
            user_request,
            output_contract,
            file_delivery_contract,
        ) {
            return (directory_lookup_text.clone(), vec![directory_lookup_text]);
        }
    }
    enforce_file_delivery_locator_contract(
        state,
        user_request,
        output_contract,
        file_delivery_contract,
        &mut normalized_text,
        &mut normalized_messages,
    );
    enforce_output_contract(
        state,
        user_request,
        output_contract,
        &mut normalized_text,
        &mut normalized_messages,
    );
    (normalized_text, normalized_messages)
}

#[cfg(test)]
fn classify_directory_lookup_input(user_request: &str) -> Option<DirectoryLookupInput> {
    let text = user_request.trim();
    if text.is_empty() {
        return None;
    }
    locator::parse_directory_lookup_input_for_tests(text)
}

#[cfg(test)]
fn classify_batch_directory_delivery_input(user_request: &str) -> Option<DirectoryLookupInput> {
    let text = user_request.trim();
    if text.is_empty() || locator::extract_directory_and_file_pair(text).is_some() {
        return None;
    }
    locator::parse_directory_lookup_input_for_tests(text)
}
#[cfg(test)]
fn resolve_file_delivery_target(
    user_request: &str,
    system_root: &std::path::Path,
    project_root: &std::path::Path,
    scan_max_depth: usize,
    scan_max_files: usize,
) -> Option<FileDeliveryTargetResolution> {
    file_delivery::resolve_file_delivery_target_from_request_for_tests(
        user_request,
        system_root,
        project_root,
        scan_max_depth,
        scan_max_files,
    )
}

#[cfg(test)]
mod tests;
