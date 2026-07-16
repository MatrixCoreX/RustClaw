use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::directory_lookup::{resolve_directory_locator_input, resolve_directory_target};
use super::locator::{
    classify_file_delivery_locator_from_hint, extract_explicit_file_path_candidates,
    normalize_locator_text,
};
#[cfg(test)]
use super::locator_test_support::classify_file_delivery_locator_input_for_tests;
use super::types::localize_delivery_message_for_request;
use super::{
    resolve_existing_dir_under_root, resolve_existing_file_under_root,
    response_has_same_file_token, trim_path_token, BatchDirectoryDeliveryResolution,
    CurrentLevelDeliveryEntries, CurrentLevelDeliveryEntriesResult, DeliveryMessageKind,
    DirectoryFileLookupResult, DirectoryLookupResolution, FileDeliveryLocatorInput,
    FileDeliveryTargetResolution, FilenameScanResult, IntentOutputContract,
};
use crate::AppState;
use crate::OutputDeliveryIntent;

// File-target resolution and batch delivery helpers used by the facade.
pub(super) fn resolve_batch_directory_delivery(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
) -> Option<BatchDirectoryDeliveryResolution> {
    if !matches!(
        output_contract.delivery_intent,
        OutputDeliveryIntent::DirectoryBatchFiles
    ) {
        return None;
    }
    let locator = resolve_directory_locator_input(
        output_contract,
        user_request,
        &state.skill_rt.workspace_root,
    )?;
    let resolved = resolve_directory_target(
        locator,
        Path::new("/"),
        &state.skill_rt.default_locator_search_dir,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
    );
    match resolved {
        DirectoryLookupResolution::Resolved(directory) => {
            match list_current_level_files_for_delivery(
                &directory,
                state.skill_rt.locator_scan_max_files,
            ) {
                CurrentLevelDeliveryEntriesResult::Ready(entries) => {
                    let subdir_hint = localize_delivery_message_for_request(
                        state,
                        DeliveryMessageKind::DirectoryHasChildDirsHint,
                        user_request,
                    );
                    let no_files_message = localize_delivery_message_for_request(
                        state,
                        DeliveryMessageKind::DirectoryNoSendableFilesInCurrentLevel,
                        user_request,
                    );
                    Some(build_batch_directory_delivery_response(
                        entries,
                        &no_files_message,
                        &subdir_hint,
                    ))
                }
                CurrentLevelDeliveryEntriesResult::UserMessage(kind) => {
                    Some(BatchDirectoryDeliveryResolution::UserMessage(
                        localize_delivery_message_for_request(state, kind, user_request),
                    ))
                }
            }
        }
        DirectoryLookupResolution::MultipleCandidates(candidates) => {
            let mut lines = Vec::with_capacity(candidates.len() + 1);
            lines.push(localize_delivery_message_for_request(
                state,
                DeliveryMessageKind::DirectoryMultipleCandidates,
                user_request,
            ));
            lines.extend(
                candidates
                    .into_iter()
                    .map(|path| path.display().to_string()),
            );
            Some(BatchDirectoryDeliveryResolution::UserMessage(
                lines.join("\n"),
            ))
        }
        DirectoryLookupResolution::UserMessage(kind) => {
            Some(BatchDirectoryDeliveryResolution::UserMessage(
                localize_delivery_message_for_request(state, kind, user_request),
            ))
        }
    }
}

pub(super) fn list_current_level_files_for_delivery(
    directory: &Path,
    max_entries: usize,
) -> CurrentLevelDeliveryEntriesResult {
    let mut entries = match std::fs::read_dir(directory) {
        Ok(v) => v
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(_) => {
            return CurrentLevelDeliveryEntriesResult::UserMessage(
                DeliveryMessageKind::DirectoryNoSendableFilesInCurrentLevel,
            )
        }
    };
    entries.sort();
    if entries.len() > max_entries.max(1) {
        return CurrentLevelDeliveryEntriesResult::UserMessage(
            DeliveryMessageKind::DirectoryEntriesTooMany,
        );
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

pub(super) fn format_batch_delivery_tokens(
    files: &[PathBuf],
    trailing_hint: Option<&str>,
) -> String {
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
    if let Some(existing_token) =
        canonical_existing_file_delivery_token(state, normalized_text, normalized_messages)
    {
        *normalized_text = existing_token.clone();
        normalized_messages.clear();
        normalized_messages.push(existing_token);
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
                    .filter(|line| {
                        matches!(
                            crate::finalize::parse_delivery_token(line),
                            Some((crate::finalize::DeliveryTokenKind::File, _))
                        )
                    })
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
        &state.skill_rt.default_locator_search_dir,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
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
        FileDeliveryTargetResolution::Candidates(paths) => {
            let mut lines = Vec::with_capacity(paths.len() + 1);
            lines.push(localize_delivery_message_for_request(
                state,
                DeliveryMessageKind::FilenameNotUnique,
                user_request,
            ));
            lines.extend(paths.into_iter().map(|path| path.display().to_string()));
            *normalized_text = lines.join("\n");
            normalized_messages
                .retain(|msg| crate::finalize::parse_delivery_file_token(msg).is_none());
        }
        FileDeliveryTargetResolution::UserMessage(msg) => {
            *normalized_text = localize_delivery_message_for_request(state, msg, user_request);
            normalized_messages
                .retain(|msg| crate::finalize::parse_delivery_file_token(msg).is_none());
        }
    }
}

fn canonical_existing_file_delivery_token(
    state: &AppState,
    normalized_text: &str,
    normalized_messages: &[String],
) -> Option<String> {
    std::iter::once(normalized_text)
        .chain(normalized_messages.iter().map(|msg| msg.as_str()))
        .find_map(|candidate| super::message_media::normalize_delivery_message(state, candidate))
        .filter(|normalized| crate::finalize::parse_delivery_file_token(normalized).is_some())
}

pub(super) fn resolve_file_delivery_target_with_hint(
    _user_request: &str,
    system_root: &Path,
    project_root: &Path,
    scan_max_depth: usize,
    scan_max_files: usize,
    locator_hint: Option<&str>,
) -> Option<FileDeliveryTargetResolution> {
    let locator_hint = normalized_locator_hint(locator_hint);
    if let Some(resolved) =
        resolve_explicit_file_path_candidate(locator_hint, None, system_root, project_root)
    {
        return Some(resolved);
    }
    let locator = locator_hint.and_then(classify_file_delivery_locator_from_hint)?;
    Some(resolve_file_delivery_locator(
        locator,
        system_root,
        project_root,
        scan_max_depth,
        scan_max_files,
    ))
}

#[cfg(test)]
pub(super) fn resolve_file_delivery_target_from_request_for_tests(
    user_request: &str,
    system_root: &Path,
    project_root: &Path,
    scan_max_depth: usize,
    scan_max_files: usize,
) -> Option<FileDeliveryTargetResolution> {
    if let Some(resolved) =
        resolve_explicit_file_path_candidate(None, Some(user_request), system_root, project_root)
    {
        return Some(resolved);
    }
    let locator = classify_file_delivery_locator_input_for_tests(user_request, None)?;
    Some(resolve_file_delivery_locator(
        locator,
        system_root,
        project_root,
        scan_max_depth,
        scan_max_files,
    ))
}

fn normalized_locator_hint(locator_hint: Option<&str>) -> Option<&str> {
    locator_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolve_file_delivery_locator(
    locator: FileDeliveryLocatorInput,
    system_root: &Path,
    project_root: &Path,
    scan_max_depth: usize,
    scan_max_files: usize,
) -> FileDeliveryTargetResolution {
    match locator {
        FileDeliveryLocatorInput::ExplicitFilePath { file_path } => {
            resolve_explicit_file_path(system_root, project_root, &file_path)
        }
        FileDeliveryLocatorInput::DirectoryAndFilename {
            directory_path,
            file_name,
        } => resolve_directory_and_file(system_root, project_root, &directory_path, &file_name),
        FileDeliveryLocatorInput::FilenameOnly { file_name } => scan_filename_under_roots(
            project_root,
            system_root,
            &file_name,
            scan_max_depth,
            scan_max_files,
        ),
    }
}

fn resolve_explicit_file_path_candidate(
    locator_hint: Option<&str>,
    user_request: Option<&str>,
    system_root: &Path,
    project_root: &Path,
) -> Option<FileDeliveryTargetResolution> {
    for source in locator_hint.into_iter().chain(user_request.into_iter()) {
        for token in extract_explicit_file_path_candidates(source) {
            let resolved = resolve_existing_file_under_root(system_root, &token)
                .or_else(|| resolve_existing_file_under_root(project_root, &token));
            if let Some(path) = resolved {
                return Some(FileDeliveryTargetResolution::Resolved(path));
            }
            if token_has_definite_file_shape(&token) {
                return Some(resolve_explicit_file_path(
                    system_root,
                    project_root,
                    &token,
                ));
            }
        }
    }
    None
}

fn token_has_definite_file_shape(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if cleaned.is_empty() || cleaned.ends_with('/') || cleaned.ends_with('\\') {
        return false;
    }
    let normalized = cleaned.replace('\\', "/");
    let last = normalized.rsplit('/').next().unwrap_or_default();
    last.contains('.')
}

pub(super) fn resolve_explicit_file_path(
    system_root: &Path,
    project_root: &Path,
    raw_path: &str,
) -> FileDeliveryTargetResolution {
    if let Some(path) = resolve_existing_file_under_root(system_root, raw_path) {
        return FileDeliveryTargetResolution::Resolved(path);
    }
    if let Some(path) = resolve_existing_file_under_root(project_root, raw_path) {
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
    let directory = resolve_existing_dir_under_root(system_root, directory_path)
        .or_else(|| resolve_existing_dir_under_root(project_root, directory_path));
    let Some(directory) = directory else {
        return FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule2DirNotFound);
    };
    match find_file_in_directory_non_recursive(&directory, file_name) {
        DirectoryFileLookupResult::Found(path) => FileDeliveryTargetResolution::Resolved(path),
        DirectoryFileLookupResult::Candidates(paths) => {
            FileDeliveryTargetResolution::Candidates(paths)
        }
        DirectoryFileLookupResult::NotFound => {
            FileDeliveryTargetResolution::UserMessage(DeliveryMessageKind::Rule2FileNotFound)
        }
    }
}

pub(super) fn scan_filename_under_roots(
    project_root: &Path,
    system_root: &Path,
    file_name: &str,
    scan_max_depth: usize,
    scan_max_files: usize,
) -> FileDeliveryTargetResolution {
    let project_outcome = scan_filename_matches_with_limit_internal(
        project_root,
        file_name,
        scan_max_depth,
        scan_max_files,
    );
    match project_outcome.result {
        FilenameScanResult::Found(path) => FileDeliveryTargetResolution::Resolved(path),
        FilenameScanResult::Candidates(paths) => {
            if let Some(path) =
                prefer_unique_direct_child_filename_candidate(project_root, &paths, file_name)
            {
                FileDeliveryTargetResolution::Resolved(path)
            } else {
                FileDeliveryTargetResolution::Candidates(paths)
            }
        }
        FilenameScanResult::TooManyEntries => {
            FileDeliveryTargetResolution::UserMessage(scan_limit_message_for_filename(file_name))
        }
        FilenameScanResult::NotFound => {
            if system_root == Path::new("/") {
                return FileDeliveryTargetResolution::UserMessage(
                    DeliveryMessageKind::Rule3FileNotFound,
                );
            }
            match scan_filename_matches_with_limit_internal(
                system_root,
                file_name,
                scan_max_depth,
                scan_max_files,
            )
            .result
            {
                FilenameScanResult::Found(path) => FileDeliveryTargetResolution::Resolved(path),
                FilenameScanResult::Candidates(paths) => {
                    FileDeliveryTargetResolution::Candidates(paths)
                }
                FilenameScanResult::NotFound => FileDeliveryTargetResolution::UserMessage(
                    DeliveryMessageKind::Rule3FileNotFound,
                ),
                FilenameScanResult::TooManyEntries => FileDeliveryTargetResolution::UserMessage(
                    scan_limit_message_for_filename(file_name),
                ),
            }
        }
    }
}

fn scan_limit_message_for_filename(file_name: &str) -> DeliveryMessageKind {
    if token_has_definite_file_shape(file_name) {
        DeliveryMessageKind::Rule3FileNotFound
    } else {
        DeliveryMessageKind::Rule3ScanTooMany
    }
}

fn prefer_unique_direct_child_filename_candidate(
    root: &Path,
    candidates: &[PathBuf],
    target: &str,
) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let want_stem_match = !target.contains('.');
    let mut direct_hits = candidates
        .iter()
        .filter_map(|path| {
            let parent = path.parent()?.to_path_buf();
            let normalized_parent = parent.canonicalize().unwrap_or(parent);
            if normalized_parent != canonical_root {
                return None;
            }
            let file_name = path.file_name()?.to_str()?;
            if file_name.eq_ignore_ascii_case(target) {
                return Some(path.clone());
            }
            if want_stem_match
                && path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case(target))
            {
                return Some(path.clone());
            }
            None
        })
        .collect::<Vec<_>>();
    direct_hits.sort();
    direct_hits.dedup();
    (direct_hits.len() == 1).then(|| direct_hits.remove(0))
}

fn ranked_candidate_paths(mut paths: Vec<PathBuf>, target: &str) -> Vec<PathBuf> {
    for path in &mut paths {
        if let Ok(canonical) = path.canonicalize() {
            *path = canonical;
        }
    }
    let target_norm = normalize_locator_text(target);
    paths.sort_by(|a, b| {
        let score_a = fuzzy_filename_candidate_score(a, &target_norm);
        let score_b = fuzzy_filename_candidate_score(b, &target_norm);
        score_b
            .cmp(&score_a)
            .then_with(|| {
                a.file_name()
                    .map(|v| v.to_string_lossy().len())
                    .unwrap_or(usize::MAX)
                    .cmp(
                        &b.file_name()
                            .map(|v| v.to_string_lossy().len())
                            .unwrap_or(usize::MAX),
                    )
            })
            .then_with(|| a.to_string_lossy().cmp(&b.to_string_lossy()))
    });
    paths.dedup();
    paths.into_iter().take(3).collect()
}

fn fuzzy_filename_candidate_score(path: &Path, target_norm: &str) -> i32 {
    let file_name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_name_norm = normalize_locator_text(&file_name);
    let stem_norm = path
        .file_stem()
        .map(|v| normalize_locator_text(&v.to_string_lossy()))
        .unwrap_or_default();
    if stem_norm == target_norm {
        return 500;
    }
    if stem_norm.starts_with(target_norm) {
        return 400;
    }
    if stem_norm.contains(target_norm) {
        return 300;
    }
    if file_name_norm.starts_with(target_norm) {
        return 200;
    }
    if file_name_norm.contains(target_norm) {
        return 100;
    }
    0
}

pub(super) fn find_file_in_directory_non_recursive(
    directory: &Path,
    file_name: &str,
) -> DirectoryFileLookupResult {
    let target = trim_path_token(file_name);
    if target.is_empty() {
        return DirectoryFileLookupResult::NotFound;
    }
    let target_norm = normalize_locator_text(&target);
    let direct = directory.join(&target);
    if let Ok(canonical) = direct.canonicalize() {
        if canonical.is_file() {
            return DirectoryFileLookupResult::Found(canonical);
        }
    }

    let mut exact_matches = Vec::new();
    let mut stem_matches = Vec::new();
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
        let file_name_norm = normalize_locator_text(&file_name);
        let is_exact = file_name_norm == target_norm;
        let is_stem_match = !target.contains('.')
            && path
                .file_stem()
                .map(|stem| stem.to_string_lossy().eq_ignore_ascii_case(&target))
                .unwrap_or(false);
        let is_fuzzy = !is_exact && file_name_norm.contains(&target_norm);
        if !is_exact && !is_stem_match && !is_fuzzy {
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
            } else if is_stem_match {
                stem_matches.push(path);
            } else {
                fuzzy_matches.push(path);
            }
        }
    }
    exact_matches.sort();
    exact_matches.dedup();
    if exact_matches.len() == 1 {
        return exact_matches
            .into_iter()
            .next()
            .and_then(|path| path.canonicalize().ok())
            .map(DirectoryFileLookupResult::Found)
            .unwrap_or(DirectoryFileLookupResult::NotFound);
    }
    if exact_matches.len() > 1 {
        return DirectoryFileLookupResult::Candidates(ranked_candidate_paths(
            exact_matches,
            &target,
        ));
    }
    stem_matches.sort();
    stem_matches.dedup();
    if stem_matches.len() == 1 {
        return stem_matches
            .into_iter()
            .next()
            .and_then(|path| path.canonicalize().ok())
            .map(DirectoryFileLookupResult::Found)
            .unwrap_or(DirectoryFileLookupResult::NotFound);
    }
    if stem_matches.len() > 1 {
        return DirectoryFileLookupResult::Candidates(ranked_candidate_paths(
            stem_matches,
            &target,
        ));
    }
    fuzzy_matches.sort();
    fuzzy_matches.dedup();
    if fuzzy_matches.is_empty() {
        DirectoryFileLookupResult::NotFound
    } else {
        DirectoryFileLookupResult::Candidates(ranked_candidate_paths(fuzzy_matches, &target))
    }
}

#[cfg(test)]
pub(crate) fn scan_filename_matches_with_limit(
    project_root: &Path,
    file_name: &str,
    max_depth: usize,
    max_files: usize,
) -> FilenameScanResult {
    scan_filename_matches_with_limit_internal(project_root, file_name, max_depth, max_files).result
}

#[derive(Debug)]
struct FilenameScanOutcome {
    result: FilenameScanResult,
}

fn scan_filename_matches_with_limit_internal(
    project_root: &Path,
    file_name: &str,
    max_depth: usize,
    max_files: usize,
) -> FilenameScanOutcome {
    if !project_root.is_dir() {
        return FilenameScanOutcome {
            result: FilenameScanResult::NotFound,
        };
    }
    let target = trim_path_token(file_name);
    if target.is_empty() {
        return FilenameScanOutcome {
            result: FilenameScanResult::NotFound,
        };
    }
    let target_norm = normalize_locator_text(&target);

    let mut exact_matches = Vec::new();
    let mut stem_matches = Vec::new();
    let mut fuzzy_matches = Vec::new();
    let mut seen_entries = 0usize;
    let mut queue = VecDeque::from([(project_root.to_path_buf(), 0usize)]);

    while let Some((dir, depth)) = queue.pop_front() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(v) => v
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        let mut child_dirs = Vec::new();
        for path in entries {
            seen_entries += 1;
            if seen_entries > max_files.max(1) {
                return FilenameScanOutcome {
                    result: FilenameScanResult::TooManyEntries,
                };
            }
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let file_type = meta.file_type();
            if file_type.is_dir() {
                if depth < max_depth {
                    child_dirs.push((path, depth + 1));
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
            let current_name_norm = normalize_locator_text(&current_name);
            if current_name_norm == target_norm {
                exact_matches.push(path);
            } else if !target.contains('.')
                && path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().eq_ignore_ascii_case(&target))
                    .unwrap_or(false)
            {
                stem_matches.push(path);
            } else if current_name_norm.contains(&target_norm) {
                fuzzy_matches.push(path);
            }
        }
        for child in child_dirs {
            queue.push_back(child);
        }
    }

    exact_matches.sort();
    exact_matches.dedup();
    if exact_matches.len() == 1 {
        return FilenameScanOutcome {
            result: exact_matches
                .into_iter()
                .next()
                .and_then(|path| path.canonicalize().ok())
                .map(FilenameScanResult::Found)
                .unwrap_or(FilenameScanResult::NotFound),
        };
    }
    if exact_matches.len() > 1 {
        if let Some(path) =
            prefer_unique_direct_child_filename_candidate(project_root, &exact_matches, &target)
        {
            return FilenameScanOutcome {
                result: FilenameScanResult::Found(path),
            };
        }
        return FilenameScanOutcome {
            result: FilenameScanResult::Candidates(ranked_candidate_paths(exact_matches, &target)),
        };
    }
    stem_matches.sort();
    stem_matches.dedup();
    if stem_matches.len() == 1 {
        return FilenameScanOutcome {
            result: stem_matches
                .into_iter()
                .next()
                .and_then(|path| path.canonicalize().ok())
                .map(FilenameScanResult::Found)
                .unwrap_or(FilenameScanResult::NotFound),
        };
    }
    if stem_matches.len() > 1 {
        if let Some(path) =
            prefer_unique_direct_child_filename_candidate(project_root, &stem_matches, &target)
        {
            return FilenameScanOutcome {
                result: FilenameScanResult::Found(path),
            };
        }
        return FilenameScanOutcome {
            result: FilenameScanResult::Candidates(ranked_candidate_paths(stem_matches, &target)),
        };
    }
    fuzzy_matches.sort();
    fuzzy_matches.dedup();
    let result = if fuzzy_matches.is_empty() {
        FilenameScanResult::NotFound
    } else {
        FilenameScanResult::Candidates(ranked_candidate_paths(fuzzy_matches, &target))
    };
    FilenameScanOutcome { result }
}
