use std::path::{Path, PathBuf};

use super::{
    localize_delivery_message, response_has_same_file_token, trim_path_token,
    BatchDirectoryDeliveryResolution, CurrentLevelDeliveryEntries,
    CurrentLevelDeliveryEntriesResult, DeliveryMessageKind, DirectoryFileLookupResult,
    DirectoryLookupResolution, FileDeliveryTargetResolution, FilenameScanResult,
    IntentOutputContract,
};
use crate::AppState;

pub(super) fn resolve_batch_directory_delivery(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
) -> Option<BatchDirectoryDeliveryResolution> {
    if !matches!(
        output_contract.delivery_intent,
        crate::intent_router::OutputDeliveryIntent::DirectoryBatchFiles
    ) {
        return None;
    }
    let locator = super::resolve_directory_locator_input(output_contract, user_request)?;
    let resolved = super::resolve_directory_target(
        locator,
        Path::new("/"),
        &state.default_locator_search_dir,
        state.locator_scan_max_depth,
        state.locator_scan_max_files,
    );
    match resolved {
        DirectoryLookupResolution::Resolved(directory) => {
            match list_current_level_files_for_delivery(&directory, state.locator_scan_max_files) {
                CurrentLevelDeliveryEntriesResult::Ready(entries) => {
                    let subdir_hint = localize_delivery_message(
                        state,
                        DeliveryMessageKind::DirectoryHasChildDirsHint,
                    );
                    let no_files_message = localize_delivery_message(
                        state,
                        DeliveryMessageKind::DirectoryNoSendableFilesInCurrentLevel,
                    );
                    Some(build_batch_directory_delivery_response(
                        entries,
                        &no_files_message,
                        &subdir_hint,
                    ))
                }
                CurrentLevelDeliveryEntriesResult::UserMessage(kind) => Some(
                    BatchDirectoryDeliveryResolution::UserMessage(localize_delivery_message(
                        state, kind,
                    )),
                ),
            }
        }
        DirectoryLookupResolution::MultipleCandidates(candidates) => {
            let mut lines = Vec::with_capacity(candidates.len() + 1);
            lines.push(localize_delivery_message(
                state,
                DeliveryMessageKind::DirectoryMultipleCandidates,
            ));
            lines.extend(candidates.into_iter().map(|path| path.display().to_string()));
            Some(BatchDirectoryDeliveryResolution::UserMessage(lines.join("\n")))
        }
        DirectoryLookupResolution::UserMessage(kind) => Some(
            BatchDirectoryDeliveryResolution::UserMessage(localize_delivery_message(state, kind)),
        ),
    }
}

pub(super) fn list_current_level_files_for_delivery(
    directory: &Path,
    max_entries: usize,
) -> CurrentLevelDeliveryEntriesResult {
    let mut entries = match std::fs::read_dir(directory) {
        Ok(v) => v.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>(),
        Err(_) => {
            return CurrentLevelDeliveryEntriesResult::UserMessage(
                DeliveryMessageKind::DirectoryNoSendableFilesInCurrentLevel,
            )
        }
    };
    entries.sort();
    if entries.len() > max_entries.max(1) {
        return CurrentLevelDeliveryEntriesResult::UserMessage(DeliveryMessageKind::DirectoryEntriesTooMany);
    }
    let mut files = Vec::new();
    let mut has_child_dirs = false;
    for path in entries {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let file_type = meta.file_type();
        if file_type.is_dir() {
            has_child_dirs = true;
            continue;
        }
        if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            if let Ok(canonical) = path.canonicalize() {
                files.push(canonical);
            } else {
                files.push(path);
            }
        }
    }
    super::dedup_and_sort_paths(&mut files);
    CurrentLevelDeliveryEntriesResult::Ready(CurrentLevelDeliveryEntries {
        files,
        has_child_dirs,
    })
}

pub(super) fn format_batch_delivery_tokens(files: &[PathBuf], trailing_hint: Option<&str>) -> String {
    let mut lines = files
        .iter()
        .map(|path| format!("FILE:{}", path.display()))
        .collect::<Vec<_>>();
    if let Some(hint) = trailing_hint.map(str::trim).filter(|hint| !hint.is_empty()) {
        lines.push(hint.to_string());
    }
    lines.join("\n")
}

pub(super) fn build_batch_directory_delivery_response(
    entries: CurrentLevelDeliveryEntries,
    no_files_message: &str,
    child_dirs_hint: &str,
) -> BatchDirectoryDeliveryResolution {
    if entries.files.is_empty() {
        let mut text = no_files_message.trim().to_string();
        if entries.has_child_dirs {
            let hint = child_dirs_hint.trim();
            if !hint.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(hint);
            }
        }
        return BatchDirectoryDeliveryResolution::UserMessage(text);
    }
    BatchDirectoryDeliveryResolution::FileTokens(format_batch_delivery_tokens(
        &entries.files,
        entries.has_child_dirs.then_some(child_dirs_hint),
    ))
}

pub(super) fn enforce_file_delivery_locator_contract(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    if !wants_file_delivery {
        return;
    }
    if let Some(batch) = resolve_batch_directory_delivery(state, user_request, output_contract) {
        match batch {
            BatchDirectoryDeliveryResolution::FileTokens(tokens_text) => {
                *normalized_text = tokens_text.clone();
                normalized_messages.clear();
                let token_lines = tokens_text
                    .lines()
                    .map(str::trim)
                    .filter(|line| line.starts_with("FILE:"))
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>();
                if token_lines.is_empty() {
                    normalized_messages.push(tokens_text);
                } else {
                    for token in token_lines {
                        if !normalized_messages.iter().any(|msg| msg == &token) {
                            normalized_messages.push(token);
                        }
                    }
                }
            }
            BatchDirectoryDeliveryResolution::UserMessage(message) => {
                *normalized_text = message.clone();
                normalized_messages.clear();
                normalized_messages.push(message);
            }
        }
        return;
    }
    let Some(resolved) = resolve_file_delivery_target_with_hint(
        user_request,
        Path::new("/"),
        &state.default_locator_search_dir,
        state.locator_scan_max_depth,
        state.locator_scan_max_files,
        Some(output_contract.locator_hint.as_str()),
    ) else {
        return;
    };
    match resolved {
        FileDeliveryTargetResolution::Resolved(path) => {
            let expected = format!("FILE:{}", path.display());
            if !response_has_same_file_token(normalized_text, normalized_messages, &path) {
                *normalized_text = expected.clone();
                if !normalized_messages.iter().any(|v| v == &expected) {
                    normalized_messages.push(expected);
                }
            }
        }
        FileDeliveryTargetResolution::UserMessage(msg) => {
            *normalized_text = localize_delivery_message(state, msg);
            normalized_messages.retain(|msg| !msg.trim_start().starts_with("FILE:"));
        }
    }
}

pub(super) fn resolve_file_delivery_target_with_hint(
    user_request: &str,
    system_root: &Path,
    project_root: &Path,
    scan_max_depth: usize,
    scan_max_files: usize,
    locator_hint: Option<&str>,
) -> Option<FileDeliveryTargetResolution> {
    let locator = super::classify_file_delivery_locator_input(user_request, locator_hint)?;
    match locator {
        super::FileDeliveryLocatorInput::ExplicitFilePath { file_path } => {
            Some(resolve_explicit_file_path(system_root, project_root, &file_path))
        }
        super::FileDeliveryLocatorInput::DirectoryAndFilename {
            directory_path,
            file_name,
        } => Some(resolve_directory_and_file(
            system_root,
            project_root,
            &directory_path,
            &file_name,
        )),
        super::FileDeliveryLocatorInput::FilenameOnly { file_name } => Some(
            scan_filename_under_project_root(project_root, &file_name, scan_max_depth, scan_max_files),
        ),
    }
}

pub(super) fn resolve_explicit_file_path(
    system_root: &Path,
    project_root: &Path,
    raw_path: &str,
) -> FileDeliveryTargetResolution {
    if let Some(path) = super::resolve_existing_file_under_root(system_root, raw_path) {
        return FileDeliveryTargetResolution::Resolved(path);
    }
    if let Some(path) = super::resolve_existing_file_under_root(project_root, raw_path) {
        return FileDeliveryTargetResolution::Resolved(path);
    }
    FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule1BothRootsMiss)
}

pub(super) fn resolve_directory_and_file(
    system_root: &Path,
    project_root: &Path,
    directory_path: &str,
    file_name: &str,
) -> FileDeliveryTargetResolution {
    let directory = super::resolve_existing_dir_under_root(system_root, directory_path)
        .or_else(|| super::resolve_existing_dir_under_root(project_root, directory_path));
    let Some(directory) = directory else {
        return FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule2DirNotFound);
    };
    match find_file_in_directory_non_recursive(&directory, file_name) {
        DirectoryFileLookupResult::Found(path) => FileDeliveryTargetResolution::Resolved(path),
        DirectoryFileLookupResult::NotFound => {
            FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule2FileNotFound)
        }
        DirectoryFileLookupResult::Multiple => {
            FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::FilenameNotUnique)
        }
    }
}

pub(super) fn scan_filename_under_project_root(
    project_root: &Path,
    file_name: &str,
    scan_max_depth: usize,
    scan_max_files: usize,
) -> FileDeliveryTargetResolution {
    match scan_filename_matches_with_limit(
        project_root,
        file_name,
        scan_max_depth,
        scan_max_files,
    ) {
        FilenameScanResult::Found(path) => FileDeliveryTargetResolution::Resolved(path),
        FilenameScanResult::NotFound => {
            FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule3FileNotFound)
        }
        FilenameScanResult::TooManyEntries => {
            FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule3ScanTooMany)
        }
        FilenameScanResult::Multiple => {
            FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::FilenameNotUnique)
        }
    }
}

pub(super) fn find_file_in_directory_non_recursive(
    directory: &Path,
    file_name: &str,
) -> DirectoryFileLookupResult {
    let target = trim_path_token(file_name);
    if target.is_empty() {
        return DirectoryFileLookupResult::NotFound;
    }
    let target_norm = super::normalize_locator_text(&target);
    let direct = directory.join(&target);
    if let Ok(canonical) = direct.canonicalize() {
        if canonical.is_file() {
            return DirectoryFileLookupResult::Found(canonical);
        }
    }

    let mut exact_matches = Vec::new();
    let mut fuzzy_matches = Vec::new();
    let entries = match std::fs::read_dir(directory) {
        Ok(v) => v,
        Err(_) => return DirectoryFileLookupResult::NotFound,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let file_name_norm = super::normalize_locator_text(&file_name);
        let is_exact = file_name_norm == target_norm;
        let is_fuzzy = !is_exact
            && (file_name_norm.contains(&target_norm) || target_norm.contains(&file_name_norm));
        if !is_exact && !is_fuzzy {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let file_type = meta.file_type();
        if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            if is_exact {
                exact_matches.push(path);
            } else {
                fuzzy_matches.push(path);
            }
        }
    }
    let mut matches = if exact_matches.is_empty() {
        fuzzy_matches
    } else {
        exact_matches
    };
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => DirectoryFileLookupResult::NotFound,
        1 => matches
            .into_iter()
            .next()
            .and_then(|path| path.canonicalize().ok())
            .map(DirectoryFileLookupResult::Found)
            .unwrap_or(DirectoryFileLookupResult::NotFound),
        _ => DirectoryFileLookupResult::Multiple,
    }
}

pub(super) fn scan_filename_matches_with_limit(
    project_root: &Path,
    file_name: &str,
    max_depth: usize,
    max_files: usize,
) -> FilenameScanResult {
    if !project_root.is_dir() {
        return FilenameScanResult::NotFound;
    }
    let target = trim_path_token(file_name);
    if target.is_empty() {
        return FilenameScanResult::NotFound;
    }
    let target_norm = super::normalize_locator_text(&target);

    let mut exact_matches = Vec::new();
    let mut fuzzy_matches = Vec::new();
    let mut seen_entries = 0usize;
    let mut stack = vec![(project_root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(v) => v.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            seen_entries += 1;
            if seen_entries > max_files.max(1) {
                return FilenameScanResult::TooManyEntries;
            }
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let file_type = meta.file_type();
            if file_type.is_dir() {
                if depth < max_depth {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !(file_type.is_file() || (file_type.is_symlink() && path.is_file())) {
                continue;
            }
            let current_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let current_name_norm = super::normalize_locator_text(&current_name);
            if current_name_norm == target_norm {
                exact_matches.push(path);
            } else if current_name_norm.contains(&target_norm)
                || target_norm.contains(&current_name_norm)
            {
                fuzzy_matches.push(path);
            }
        }
    }

    let mut matches = if exact_matches.is_empty() {
        fuzzy_matches
    } else {
        exact_matches
    };
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => FilenameScanResult::NotFound,
        1 => matches
            .into_iter()
            .next()
            .and_then(|path| path.canonicalize().ok())
            .map(FilenameScanResult::Found)
            .unwrap_or(FilenameScanResult::NotFound),
        _ => FilenameScanResult::Multiple,
    }
}
