use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rustclaw_fs_discovery::{
    discover, resolve_root, BackendPreference, Completeness, DiscoveryBackend, DiscoveryBudget,
    DiscoveryPolicy, DiscoveryRequest, DiscoverySelector, TargetKind,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct ScanLimits {
    pub(super) max_depth: Option<usize>,
    pub(super) hard_entry_limit: usize,
    pub(super) include_hidden: bool,
    pub(super) respect_ignore: bool,
    pub(super) deadline: Option<Duration>,
    pub(super) backend: BackendPreference,
}

#[derive(Debug, Clone)]
pub(super) struct WalkStats {
    pub(super) visited_files: usize,
    pub(super) visited_directories: usize,
    pub(super) skipped_ignored: usize,
    pub(super) skipped_hidden: usize,
    pub(super) skipped_symlinks: usize,
    pub(super) permission_denied: usize,
    pub(super) skipped_counts_complete: bool,
    pub(super) completeness: Completeness,
    pub(super) limit_reached: bool,
    pub(super) backend: DiscoveryBackend,
    pub(super) backend_version: Option<String>,
    pub(super) backend_fallback_reason: Option<String>,
    pub(super) backend_elapsed_ms: u64,
}

impl WalkStats {
    pub(super) fn mark_hard_limit(&mut self) {
        self.completeness = Completeness::PartialHardLimit;
        self.limit_reached = true;
    }
}

fn discovery_boundary(path: &Path) -> PathBuf {
    let workspace = workspace_root();
    if path.starts_with(&workspace) {
        return workspace;
    }
    #[cfg(test)]
    {
        return path.to_path_buf();
    }
    #[cfg(not(test))]
    workspace
}

fn walk_with_kind(
    path: &Path,
    limits: ScanLimits,
    target_kind: TargetKind,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    let mut selector = DiscoverySelector::default();
    selector.target_kind = target_kind;
    walk_with_selector(path, limits, selector, f)
}

fn walk_with_selector(
    path: &Path,
    limits: ScanLimits,
    selector: DiscoverySelector,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    let boundary = discovery_boundary(path);
    let mut request = DiscoveryRequest::new(&boundary, path);
    request.selector = selector;
    request.policy = DiscoveryPolicy {
        include_hidden: limits.include_hidden,
        respect_ignore: limits.respect_ignore,
    };
    request.budget = DiscoveryBudget {
        max_depth: limits.max_depth,
        hard_entry_limit: limits.hard_entry_limit,
        match_snapshot_limit: limits.hard_entry_limit,
        deadline: limits.deadline,
        cancellation: None,
    };
    request.backend = limits.backend;
    let report = discover(&request).map_err(|error| error.to_string())?;
    let mut inspected = 0usize;
    let mut stopped = false;
    for entry in &report.entries {
        inspected = inspected.saturating_add(1);
        if f(&entry.path) {
            stopped = true;
            break;
        }
    }
    let mut stats = WalkStats {
        visited_files: report.visited_files,
        visited_directories: report.visited_directories,
        skipped_ignored: report.skipped_ignored,
        skipped_hidden: report.skipped_hidden,
        skipped_symlinks: report.skipped_symlinks,
        permission_denied: report.permission_denied,
        skipped_counts_complete: report.skipped_counts_complete,
        completeness: report.completeness,
        limit_reached: report.scan_truncated(),
        backend: report.backend.backend,
        backend_version: report.backend.version,
        backend_fallback_reason: report.backend.fallback_reason,
        backend_elapsed_ms: report.backend.elapsed_ms,
    };
    if stopped && inspected < report.entries.len() {
        stats.mark_hard_limit();
    }
    Ok(stats)
}

pub(super) fn walk_collect_selected(
    path: &Path,
    limits: ScanLimits,
    selector: DiscoverySelector,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    walk_with_selector(path, limits, selector, f)
}

pub(super) fn walk_collect(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    walk_with_kind(path, limits, TargetKind::File, f)
}

pub(super) fn to_rel(root: &Path, path: &Path) -> String {
    rustclaw_fs_discovery::relative_path(root, path)
}

pub(super) fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(super) fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let raw = Path::new(input);
    if raw
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err("path with '..' is not allowed".to_string());
    }
    match resolve_root(workspace_root, raw) {
        Ok((_, root)) => Ok(root),
        Err(error) => {
            #[cfg(test)]
            {
                let workspace_is_current = workspace_root
                    .canonicalize()
                    .ok()
                    .zip(
                        std::env::current_dir()
                            .ok()
                            .and_then(|value| value.canonicalize().ok()),
                    )
                    .is_some_and(|(workspace, current)| workspace == current);
                if workspace_is_current
                    && raw.is_absolute()
                    && raw.starts_with(std::env::temp_dir())
                {
                    return raw
                        .canonicalize()
                        .map_err(|io_error| format!("path resolution failed: {io_error}"));
                }
            }
            Err(error.to_string())
        }
    }
}
