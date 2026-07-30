use std::path::{Path, PathBuf};
use std::time::Duration;

use fs_discovery::{
    discover, Completeness, DiscoveryBudget, DiscoveryPolicy, DiscoveryRequest, DiscoverySelector,
    MatchMode, TargetKind,
};

use super::locator::{directory_lookup_input_from_hint, normalize_locator_text};
use super::types::localize_delivery_message_for_request;
use super::{
    dedup_and_sort_paths, resolve_existing_dir_under_root, DeliveryMessageKind,
    DirectoryEntriesListResult, DirectoryLookupInput, DirectoryLookupResolution,
    IntentOutputContract,
};
use crate::AppState;
use crate::{OutputDeliveryIntent, OutputLocatorKind};

const LOCATOR_DEADLINE: Duration = Duration::from_secs(5);
const DIRECT_DIRECTORY_ENTRY_LIMIT: usize = 1_000;

struct DirectoryCandidateScan {
    paths: Vec<PathBuf>,
    completeness: Completeness,
}

// Directory-only lookup and listing flow used by delivery interception.
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
    let request = resolve_directory_locator_input(
        output_contract,
        user_request,
        &state.skill_rt.workspace_root,
    )?;
    let resolved = resolve_directory_target(
        request,
        Path::new("/"),
        &state.skill_rt.default_locator_search_dir,
    );
    match resolved {
        DirectoryLookupResolution::Resolved(directory) => {
            match list_directory_entries_for_user(&directory, DIRECT_DIRECTORY_ENTRY_LIMIT) {
                DirectoryEntriesListResult::FilePaths(paths) => {
                    if paths.is_empty() {
                        Some(localize_delivery_message_for_request(
                            state,
                            DeliveryMessageKind::DirectoryNoFilesInCurrentLevel,
                            user_request,
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
                DirectoryEntriesListResult::UserMessage(kind) => Some(
                    localize_delivery_message_for_request(state, kind, user_request),
                ),
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
            Some(lines.join("\n"))
        }
        DirectoryLookupResolution::UserMessage(kind) => Some(
            localize_delivery_message_for_request(state, kind, user_request),
        ),
    }
}

pub(super) fn resolve_directory_locator_input(
    output_contract: &IntentOutputContract,
    _user_request: &str,
    workspace_root: &Path,
) -> Option<DirectoryLookupInput> {
    if matches!(
        output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    ) {
        return Some(DirectoryLookupInput::ExplicitPath {
            directory_path: workspace_root
                .canonicalize()
                .unwrap_or_else(|_| workspace_root.to_path_buf())
                .display()
                .to_string(),
        });
    }
    if let Some(from_hint) = directory_lookup_input_from_hint(&output_contract.locator_hint) {
        return Some(from_hint);
    }
    None
}

pub(super) fn resolve_directory_target(
    input: DirectoryLookupInput,
    system_root: &Path,
    project_root: &Path,
) -> DirectoryLookupResolution {
    match input {
        DirectoryLookupInput::ExplicitPath { directory_path } => {
            if let Some(directory) = resolve_existing_dir_under_root(system_root, &directory_path) {
                return DirectoryLookupResolution::Resolved(directory);
            }
            if let Some(directory) = resolve_existing_dir_under_root(project_root, &directory_path)
            {
                return DirectoryLookupResolution::Resolved(directory);
            }
            DirectoryLookupResolution::UserMessage(DeliveryMessageKind::DirectoryBothRootsMiss)
        }
        DirectoryLookupInput::NameHint { directory_hint } => {
            let system_exact = scan_directory_candidates(system_root, &directory_hint, true);
            let project_exact = scan_directory_candidates(project_root, &directory_hint, true);
            let exact_complete =
                system_exact.completeness.is_complete() && project_exact.completeness.is_complete();
            let mut exact = system_exact.paths;
            exact.extend(project_exact.paths);
            dedup_and_sort_paths(&mut exact);
            if !exact_complete {
                return DirectoryLookupResolution::UserMessage(
                    DeliveryMessageKind::DirectoryEntriesTooMany,
                );
            }
            if exact.len() == 1 {
                return DirectoryLookupResolution::Resolved(exact[0].clone());
            }
            if exact.len() > 1 {
                return DirectoryLookupResolution::MultipleCandidates(
                    exact.into_iter().take(3).collect(),
                );
            }

            let system_fuzzy = scan_directory_candidates(system_root, &directory_hint, false);
            let project_fuzzy = scan_directory_candidates(project_root, &directory_hint, false);
            let fuzzy_complete =
                system_fuzzy.completeness.is_complete() && project_fuzzy.completeness.is_complete();
            let mut fuzzy = system_fuzzy.paths;
            fuzzy.extend(project_fuzzy.paths);
            dedup_and_sort_paths(&mut fuzzy);
            if !fuzzy_complete {
                return DirectoryLookupResolution::UserMessage(
                    DeliveryMessageKind::DirectoryEntriesTooMany,
                );
            }
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

#[cfg(test)]
pub(super) fn collect_directory_candidates(
    root: &Path,
    hint: &str,
    exact_only: bool,
) -> Vec<PathBuf> {
    scan_directory_candidates(root, hint, exact_only).paths
}

fn scan_directory_candidates(root: &Path, hint: &str, exact_only: bool) -> DirectoryCandidateScan {
    // Never turn a missing hint into an unbounded scan of the host root. Explicit
    // paths are still resolved directly above; production name lookup searches
    // the focused locator root instead.
    if !root.is_dir() || root == Path::new("/") {
        return DirectoryCandidateScan {
            paths: Vec::new(),
            completeness: Completeness::Complete,
        };
    }
    let hint_norm = normalize_locator_text(hint);
    if hint_norm.is_empty() {
        return DirectoryCandidateScan {
            paths: Vec::new(),
            completeness: Completeness::Complete,
        };
    }
    let mut request = DiscoveryRequest::new(root, root);
    request.selector = DiscoverySelector {
        patterns: vec![hint_norm],
        target_kind: TargetKind::Directory,
        match_mode: if exact_only {
            MatchMode::Exact
        } else {
            MatchMode::Contains
        },
        ..DiscoverySelector::default()
    };
    request.policy = DiscoveryPolicy::default();
    request.budget = DiscoveryBudget {
        deadline: Some(LOCATOR_DEADLINE),
        ..DiscoveryBudget::default()
    };
    match discover(&request) {
        Ok(report) => DirectoryCandidateScan {
            completeness: report.completeness,
            paths: report
                .entries
                .into_iter()
                .filter_map(|entry| entry.path.canonicalize().ok())
                .collect(),
        },
        Err(_) => DirectoryCandidateScan {
            paths: Vec::new(),
            completeness: Completeness::PartialPermission,
        },
    }
}

pub(super) fn list_directory_entries_for_user(
    directory: &Path,
    max_entries: usize,
) -> DirectoryEntriesListResult {
    let mut entries = match std::fs::read_dir(directory) {
        Ok(v) => v
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(_) => return DirectoryEntriesListResult::FilePaths(Vec::new()),
    };
    entries.sort();
    if entries.len() > max_entries.max(1) {
        return DirectoryEntriesListResult::UserMessage(
            DeliveryMessageKind::DirectoryEntriesTooMany,
        );
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
