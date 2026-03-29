use std::path::{Path, PathBuf};

use super::{
    dedup_and_sort_paths, directory_hint_from_token, directory_lookup_input_from_hint,
    extract_directory_and_file_pair, extract_directory_name_hint,
    extract_directory_path_candidate_from_request, list_current_level_files_for_delivery,
    localize_delivery_message, looks_like_directory_path_hint, looks_like_directory_token,
    normalize_locator_text, DeliveryMessageKind, DirectoryEntriesListResult, DirectoryLookupInput,
    DirectoryLookupResolution, IntentOutputContract, OutputDeliveryIntent,
};
use crate::AppState;

pub(super) fn try_handle_directory_lookup_request(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
    file_delivery_contract: bool,
) -> Option<String> {
    let allow_directory_lookup = matches!(
        output_contract.delivery_intent,
        OutputDeliveryIntent::DirectoryLookup
    );
    if !allow_directory_lookup || file_delivery_contract {
        return None;
    }
    let request = resolve_directory_locator_input(output_contract, user_request)?;
    let resolved = resolve_directory_target(
        request,
        Path::new("/"),
        &state.default_locator_search_dir,
        state.locator_scan_max_depth,
        state.locator_scan_max_files,
    );
    match resolved {
        DirectoryLookupResolution::Resolved(directory) => {
            match list_directory_entries_for_user(&directory, state.locator_scan_max_files) {
                DirectoryEntriesListResult::FilePaths(paths) => {
                    if paths.is_empty() {
                        Some(localize_delivery_message(
                            state,
                            DeliveryMessageKind::DirectoryNoFilesInCurrentLevel,
                        ))
                    } else {
                        Some(
                            paths
                                .iter()
                                .map(|path| path.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    }
                }
                DirectoryEntriesListResult::UserMessage(kind) => {
                    Some(localize_delivery_message(state, kind))
                }
            }
        }
        DirectoryLookupResolution::MultipleCandidates(candidates) => {
            let mut lines = Vec::with_capacity(candidates.len() + 1);
            lines.push(localize_delivery_message(
                state,
                DeliveryMessageKind::DirectoryMultipleCandidates,
            ));
            lines.extend(candidates.into_iter().map(|path| path.display().to_string()));
            Some(lines.join("\n"))
        }
        DirectoryLookupResolution::UserMessage(kind) => Some(localize_delivery_message(state, kind)),
    }
}

pub(super) fn resolve_directory_locator_input(
    output_contract: &IntentOutputContract,
    user_request: &str,
) -> Option<DirectoryLookupInput> {
    if let Some(from_hint) = directory_lookup_input_from_hint(&output_contract.locator_hint) {
        return Some(from_hint);
    }
    parse_directory_lookup_input(user_request.trim())
}

pub(super) fn parse_directory_lookup_input(text: &str) -> Option<DirectoryLookupInput> {
    if let Some(path) = extract_directory_path_candidate_from_request(text) {
        return Some(DirectoryLookupInput::ExplicitPath { directory_path: path });
    }

    if let Some(hint) = extract_directory_name_hint(text) {
        return directory_lookup_input_from_hint(&hint);
    }
    None
}

pub(super) fn resolve_directory_target(
    input: DirectoryLookupInput,
    system_root: &Path,
    project_root: &Path,
    max_depth: usize,
    max_scan_entries: usize,
) -> DirectoryLookupResolution {
    match input {
        DirectoryLookupInput::ExplicitPath { directory_path } => {
            if let Some(directory) = super::resolve_existing_dir_under_root(system_root, &directory_path) {
                return DirectoryLookupResolution::Resolved(directory);
            }
            if let Some(directory) = super::resolve_existing_dir_under_root(project_root, &directory_path) {
                return DirectoryLookupResolution::Resolved(directory);
            }
            DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
        }
        DirectoryLookupInput::NameHint { directory_hint } => {
            let mut exact = collect_directory_candidates(
                system_root,
                &directory_hint,
                max_depth,
                max_scan_entries,
                true,
            );
            let mut project_exact = collect_directory_candidates(
                project_root,
                &directory_hint,
                max_depth,
                max_scan_entries,
                true,
            );
            exact.append(&mut project_exact);
            dedup_and_sort_paths(&mut exact);
            if exact.len() == 1 {
                return DirectoryLookupResolution::Resolved(exact[0].clone());
            }
            if exact.len() > 1 {
                return DirectoryLookupResolution::MultipleCandidates(exact.into_iter().take(3).collect());
            }

            let mut fuzzy = collect_directory_candidates(
                system_root,
                &directory_hint,
                max_depth,
                max_scan_entries,
                false,
            );
            let mut project_fuzzy = collect_directory_candidates(
                project_root,
                &directory_hint,
                max_depth,
                max_scan_entries,
                false,
            );
            fuzzy.append(&mut project_fuzzy);
            dedup_and_sort_paths(&mut fuzzy);
            if fuzzy.len() == 1 {
                DirectoryLookupResolution::Resolved(fuzzy[0].clone())
            } else if fuzzy.len() > 1 {
                DirectoryLookupResolution::MultipleCandidates(fuzzy.into_iter().take(3).collect())
            } else {
                DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
            }
        }
    }
}

pub(super) fn collect_directory_candidates(
    root: &Path,
    hint: &str,
    max_depth: usize,
    max_scan_entries: usize,
    exact_only: bool,
) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }
    let hint_norm = normalize_locator_text(hint);
    if hint_norm.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut scanned = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(v) => v.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            scanned += 1;
            if scanned > max_scan_entries.max(1) {
                return out;
            }
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !meta.file_type().is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let name_norm = normalize_locator_text(&name);
            let matched = if exact_only {
                name_norm == hint_norm
            } else {
                name_norm == hint_norm
                    || name_norm.contains(&hint_norm)
                    || hint_norm.contains(&name_norm)
            };
            if matched {
                if let Ok(canonical) = path.canonicalize() {
                    out.push(canonical);
                } else {
                    out.push(path.clone());
                }
            }
            if depth < max_depth {
                stack.push((path, depth + 1));
            }
        }
    }
    out
}

pub(super) fn list_directory_entries_for_user(
    directory: &Path,
    max_entries: usize,
) -> DirectoryEntriesListResult {
    let mut entries = match std::fs::read_dir(directory) {
        Ok(v) => v.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>(),
        Err(_) => return DirectoryEntriesListResult::FilePaths(Vec::new()),
    };
    entries.sort();
    if entries.len() > max_entries.max(1) {
        return DirectoryEntriesListResult::UserMessage(DeliveryMessageKind::DirectoryEntriesTooMany);
    }
    let mut files = Vec::new();
    for path in entries {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let file_type = meta.file_type();
        if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            if let Ok(canonical) = path.canonicalize() {
                files.push(canonical);
            } else {
                files.push(path);
            }
        }
    }
    dedup_and_sort_paths(&mut files);
    DirectoryEntriesListResult::FilePaths(files)
}
