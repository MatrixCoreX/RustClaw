use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub(super) struct ScanLimits {
    pub(super) max_depth: usize,
    pub(super) max_files: usize,
}

#[derive(Debug, Default)]
pub(super) struct WalkStats {
    pub(super) visited: usize,
    pub(super) limit_reached: bool,
}

pub(super) fn skip_default_scan_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".venv" | "venv" | "__pycache__"
    )
}

pub(super) fn walk_collect(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    let mut stats = WalkStats::default();
    let mut stop = false;
    walk_collect_inner(path, 0, limits, &mut stats, &mut stop, f)?;
    Ok(stats)
}

fn walk_collect_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    stats: &mut WalkStats,
    stop: &mut bool,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if *stop {
        return Ok(());
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|err| format!("metadata failed: {err}"))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        if stats.visited >= limits.max_files {
            stats.limit_reached = true;
            return Ok(());
        }
        stats.visited += 1;
        if f(path) {
            *stop = true;
        }
        return Ok(());
    }
    if !metadata.is_dir() || depth > limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("entry type failed: {err}"))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !skip_default_scan_dir(&path) {
                dirs.push(path);
            }
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    files.sort();
    dirs.sort();
    for path in files {
        if stats.visited >= limits.max_files {
            stats.limit_reached = true;
            return Ok(());
        }
        stats.visited += 1;
        if f(&path) {
            *stop = true;
            return Ok(());
        }
    }
    for path in dirs {
        if *stop {
            return Ok(());
        }
        if depth < limits.max_depth {
            walk_collect_inner(&path, depth + 1, limits, stats, stop, f)?;
        }
    }
    Ok(())
}

pub(super) fn walk_collect_nodes(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    let mut stats = WalkStats::default();
    let mut stop = false;
    walk_collect_nodes_inner(path, 0, limits, &mut stats, &mut stop, f)?;
    Ok(stats)
}

pub(super) fn walk_collect_dirs(
    path: &Path,
    limits: ScanLimits,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<WalkStats, String> {
    let mut stats = WalkStats::default();
    let mut stop = false;
    walk_collect_dirs_inner(path, 0, limits, &mut stats, &mut stop, f)?;
    Ok(stats)
}

fn walk_collect_dirs_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    stats: &mut WalkStats,
    stop: &mut bool,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if *stop {
        return Ok(());
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|err| format!("metadata failed: {err}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    if stats.visited >= limits.max_files {
        stats.limit_reached = true;
        return Ok(());
    }
    stats.visited += 1;
    if f(path) {
        *stop = true;
        return Ok(());
    }
    if depth >= limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    let mut dirs = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("entry type failed: {err}"))?;
        if file_type.is_dir() && !file_type.is_symlink() && !skip_default_scan_dir(&path) {
            dirs.push(path);
        }
    }
    dirs.sort();
    for path in dirs {
        if *stop {
            return Ok(());
        }
        walk_collect_dirs_inner(&path, depth + 1, limits, stats, stop, f)?;
    }
    Ok(())
}

fn walk_collect_nodes_inner(
    path: &Path,
    depth: usize,
    limits: ScanLimits,
    stats: &mut WalkStats,
    stop: &mut bool,
    f: &mut dyn FnMut(&Path) -> bool,
) -> Result<(), String> {
    if *stop {
        return Ok(());
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|err| format!("metadata failed: {err}"))?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        if stats.visited >= limits.max_files {
            stats.limit_reached = true;
            return Ok(());
        }
        stats.visited += 1;
        if f(path) {
            *stop = true;
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    if f(path) {
        *stop = true;
        return Ok(());
    }
    if depth > limits.max_depth {
        return Ok(());
    }
    let iter = std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))?;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("entry type failed: {err}"))?;
        if file_type.is_dir() && !file_type.is_symlink() {
            if !skip_default_scan_dir(&path) {
                dirs.push(path);
            }
        } else {
            files.push(path);
        }
    }
    files.sort();
    dirs.sort();
    for path in files {
        if stats.visited >= limits.max_files {
            stats.limit_reached = true;
            return Ok(());
        }
        stats.visited += 1;
        if f(&path) {
            *stop = true;
            return Ok(());
        }
    }
    for path in dirs {
        if *stop {
            return Ok(());
        }
        if depth < limits.max_depth {
            walk_collect_nodes_inner(&path, depth + 1, limits, stats, stop, f)?;
        }
    }
    Ok(())
}

pub(super) fn path_kind(path: &Path) -> &'static str {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => "symlink",
        Ok(metadata) if metadata.is_dir() => "dir",
        Ok(metadata) if metadata.is_file() => "file",
        _ => "other",
    }
}

pub(super) fn to_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
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
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace_root.join(raw)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|err| format!("path resolution failed: {err}"))?;
    let workspace = workspace_root
        .canonicalize()
        .map_err(|err| format!("workspace resolution failed: {err}"))?;
    if resolved.starts_with(&workspace) || test_fixture_path_allowed(&workspace, &resolved) {
        return Ok(resolved);
    }
    Err("path is outside workspace".to_string())
}

#[cfg(test)]
fn test_fixture_path_allowed(workspace: &Path, path: &Path) -> bool {
    let current = std::env::current_dir()
        .ok()
        .and_then(|value| value.canonicalize().ok());
    current.as_deref() == Some(workspace) && path.starts_with(std::env::temp_dir())
}

#[cfg(not(test))]
fn test_fixture_path_allowed(_workspace: &Path, _path: &Path) -> bool {
    false
}
