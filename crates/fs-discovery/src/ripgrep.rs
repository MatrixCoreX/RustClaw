use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::ripgrep_process::{base_command, resolve_binary, run_bounded, RipgrepBinary};
use crate::{
    matcher::CompiledSelector, relative_path, resolve_root, BackendProvenance, Completeness,
    DiscoveryBackend, DiscoveryEntry, DiscoveryReport, DiscoveryRequest, EntryKind, RipgrepStatus,
};

const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) struct RipgrepFailure {
    pub(crate) reason_code: String,
    pub(crate) unavailable: bool,
}

pub(crate) fn status() -> RipgrepStatus {
    crate::ripgrep_process::status()
}

pub(crate) fn discover(request: &DiscoveryRequest) -> Result<DiscoveryReport, RipgrepFailure> {
    let started = Instant::now();
    let binary = resolve_binary().map_err(|reason_code| RipgrepFailure {
        reason_code,
        unavailable: true,
    })?;
    let (workspace_root, root) =
        resolve_root(&request.workspace_root, &request.root).map_err(|error| RipgrepFailure {
            reason_code: error.code().to_string(),
            unavailable: false,
        })?;
    if root.is_file() {
        return discover_single_file(request, workspace_root, root, binary, started);
    }
    let selector = CompiledSelector::new(&request.selector).map_err(|error| RipgrepFailure {
        reason_code: error.code().to_string(),
        unavailable: false,
    })?;
    let mut command = base_command(binary, &root);
    command
        .arg("--files")
        .arg("--null")
        .arg("--color=never")
        .arg("--no-messages")
        .arg("--no-require-git");
    if request.policy.include_hidden {
        command.arg("--hidden");
    }
    if !request.policy.respect_ignore {
        command.arg("--no-ignore");
    }
    if let Some(max_depth) = request.budget.max_depth {
        command.arg("--max-depth").arg(max_depth.to_string());
    }
    command.arg(".");

    let captured =
        run_bounded(command, &request.budget, MAX_CAPTURE_BYTES, false).map_err(|reason_code| {
            RipgrepFailure {
                reason_code,
                unavailable: false,
            }
        })?;
    let hard_entry_limit = request.budget.hard_entry_limit.max(1);
    let snapshot_limit = request.budget.match_snapshot_limit.max(1);
    let mut visited_files = 0usize;
    let mut entries = Vec::new();
    let mut completeness = if captured.cancelled || captured.timed_out {
        Completeness::PartialDeadline
    } else if captured.output_truncated {
        Completeness::PartialHardLimit
    } else {
        Completeness::Complete
    };
    let mut raw_paths = captured
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|raw| PathBuf::from(String::from_utf8_lossy(raw).into_owned()))
        .collect::<Vec<_>>();
    raw_paths.sort();
    raw_paths.dedup();
    for raw_relative in raw_paths {
        if visited_files >= hard_entry_limit {
            completeness = Completeness::PartialHardLimit;
            break;
        }
        visited_files = visited_files.saturating_add(1);
        let Some(relative) = normalize_relative_output_path(&raw_relative) else {
            continue;
        };
        let path = root.join(&relative);
        if !path.starts_with(&root) || !path.is_file() || !selector.matches(&path, &relative) {
            continue;
        }
        if entries.len() >= snapshot_limit {
            completeness = Completeness::PartialHardLimit;
            break;
        }
        entries.push(DiscoveryEntry {
            relative_path: relative_path(&workspace_root, &path),
            path,
            kind: EntryKind::File,
        });
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    entries.dedup_by(|left, right| left.relative_path == right.relative_path);
    Ok(DiscoveryReport {
        workspace_root,
        root,
        entries,
        completeness,
        visited_files,
        visited_directories: 0,
        skipped_ignored: 0,
        skipped_hidden: 0,
        skipped_symlinks: 0,
        permission_denied: 0,
        skipped_counts_complete: false,
        cancelled: captured.cancelled,
        traversal_start: 0,
        traversal_next: (!completeness.is_complete()).then_some(visited_files),
        backend: BackendProvenance {
            backend: DiscoveryBackend::Ripgrep,
            version: Some(binary.version.clone()),
            fallback_reason: None,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        },
    })
}

fn normalize_relative_output_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn discover_single_file(
    request: &DiscoveryRequest,
    workspace_root: PathBuf,
    root: PathBuf,
    binary: &RipgrepBinary,
    started: Instant,
) -> Result<DiscoveryReport, RipgrepFailure> {
    let selector = CompiledSelector::new(&request.selector).map_err(|error| RipgrepFailure {
        reason_code: error.code().to_string(),
        unavailable: false,
    })?;
    let entries = selector
        .matches(&root, Path::new(root.file_name().unwrap_or_default()))
        .then(|| DiscoveryEntry {
            relative_path: relative_path(&workspace_root, &root),
            path: root.clone(),
            kind: EntryKind::File,
        })
        .into_iter()
        .collect();
    Ok(DiscoveryReport {
        workspace_root,
        root,
        entries,
        completeness: Completeness::Complete,
        visited_files: 1,
        visited_directories: 0,
        skipped_ignored: 0,
        skipped_hidden: 0,
        skipped_symlinks: 0,
        permission_denied: 0,
        skipped_counts_complete: false,
        cancelled: false,
        traversal_start: 0,
        traversal_next: None,
        backend: BackendProvenance {
            backend: DiscoveryBackend::Ripgrep,
            version: Some(binary.version.clone()),
            fallback_reason: None,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        },
    })
}
