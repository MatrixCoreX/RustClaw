use std::io::ErrorKind;
use std::path::Path;
use std::time::Instant;

use ignore::WalkBuilder;

use crate::{
    matcher::CompiledSelector, relative_path, resolve_root, BackendProvenance, Completeness,
    DiscoveryEntry, DiscoveryError, DiscoveryReport, DiscoveryRequest, DiscoverySelector,
    EntryKind, TargetKind,
};

pub fn normalize_name(text: &str) -> String {
    text.trim()
        .chars()
        .map(|ch| match ch {
            '／' | '＼' | '、' => '/',
            '－' => '-',
            '＿' => '_',
            '．' | '。' => '.',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            '｛' => '{',
            '｝' => '}',
            '　' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .to_lowercase()
}

fn entry_kind(path: &Path, file_type: Option<ignore::DirEntry>) -> EntryKind {
    let file_type = file_type.and_then(|entry| entry.file_type());
    match file_type {
        Some(value) if value.is_file() => EntryKind::File,
        Some(value) if value.is_dir() => EntryKind::Directory,
        _ => match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => EntryKind::File,
            Ok(metadata) if metadata.is_dir() => EntryKind::Directory,
            _ => EntryKind::Other,
        },
    }
}

fn target_kind_matches(selector: &DiscoverySelector, kind: EntryKind) -> bool {
    matches!(selector.target_kind, TargetKind::Any)
        || matches!(
            (selector.target_kind, kind),
            (TargetKind::File, EntryKind::File)
        )
        || matches!(
            (selector.target_kind, kind),
            (TargetKind::Directory, EntryKind::Directory)
        )
}

fn deadline_reached(request: &DiscoveryRequest, started: Instant) -> bool {
    request
        .budget
        .deadline
        .is_some_and(|deadline| started.elapsed() >= deadline)
}

fn cancelled(request: &DiscoveryRequest) -> bool {
    request
        .budget
        .cancellation
        .as_ref()
        .is_some_and(|token| token.is_cancelled())
}

pub(crate) fn discover_rust(
    request: &DiscoveryRequest,
    fallback_reason: Option<String>,
) -> Result<DiscoveryReport, DiscoveryError> {
    let backend_started = Instant::now();
    let (workspace_root, root) = resolve_root(&request.workspace_root, &request.root)?;
    let selector = CompiledSelector::new(&request.selector)?;
    let mut builder = WalkBuilder::new(&root);
    builder
        .follow_links(false)
        .hidden(!request.policy.include_hidden)
        .ignore(request.policy.respect_ignore)
        .git_ignore(request.policy.respect_ignore)
        .git_global(request.policy.respect_ignore)
        .git_exclude(request.policy.respect_ignore)
        .parents(request.policy.respect_ignore)
        .require_git(false)
        .max_depth(request.budget.max_depth);

    let hard_entry_limit = request.budget.hard_entry_limit.max(1);
    let match_snapshot_limit = request.budget.match_snapshot_limit.max(1);
    let started = Instant::now();
    let mut entries = Vec::new();
    let mut completeness = Completeness::Complete;
    let mut visited_files = 0usize;
    let mut visited_directories = 0usize;
    let mut skipped_symlinks = 0usize;
    let mut permission_denied = 0usize;
    let mut was_cancelled = false;
    let mut traversed_entries = 0usize;

    for item in builder.build() {
        if deadline_reached(request, started) || cancelled(request) {
            completeness = Completeness::PartialDeadline;
            was_cancelled = cancelled(request);
            break;
        }
        let entry = match item {
            Ok(entry) => entry,
            Err(error) => {
                if error
                    .io_error()
                    .is_some_and(|io_error| io_error.kind() == ErrorKind::PermissionDenied)
                {
                    permission_denied = permission_denied.saturating_add(1);
                    if completeness.is_complete() {
                        completeness = Completeness::PartialPermission;
                    }
                }
                continue;
            }
        };
        if entry.path_is_symlink() {
            skipped_symlinks = skipped_symlinks.saturating_add(1);
            continue;
        }
        traversed_entries = traversed_entries.saturating_add(1);
        if traversed_entries <= request.budget.start_after_entries {
            continue;
        }
        if visited_files.saturating_add(visited_directories) >= hard_entry_limit {
            completeness = Completeness::PartialHardLimit;
            break;
        }
        let kind = entry_kind(entry.path(), Some(entry.clone()));
        match kind {
            EntryKind::File => visited_files = visited_files.saturating_add(1),
            EntryKind::Directory => visited_directories = visited_directories.saturating_add(1),
            EntryKind::Other => continue,
        }
        let relative_to_root = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        if target_kind_matches(&request.selector, kind)
            && selector.matches(entry.path(), relative_to_root)
        {
            if entries.len() >= match_snapshot_limit {
                completeness = Completeness::PartialHardLimit;
                break;
            }
            entries.push(DiscoveryEntry {
                path: entry.path().to_path_buf(),
                relative_path: relative_path(&workspace_root, entry.path()),
                kind,
            });
        }
    }
    entries.sort_by(|left, right| {
        left.path
            .components()
            .count()
            .cmp(&right.path.components().count())
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    entries.dedup_by(|left, right| left.relative_path == right.relative_path);

    Ok(DiscoveryReport {
        workspace_root,
        root,
        entries,
        completeness,
        visited_files,
        visited_directories,
        skipped_ignored: 0,
        skipped_hidden: 0,
        skipped_symlinks,
        permission_denied,
        skipped_counts_complete: false,
        cancelled: was_cancelled,
        traversal_start: request.budget.start_after_entries,
        traversal_next: (!completeness.is_complete()).then_some(
            request
                .budget
                .start_after_entries
                .saturating_add(visited_files)
                .saturating_add(visited_directories),
        ),
        backend: BackendProvenance::rust(
            backend_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            fallback_reason,
        ),
    })
}
