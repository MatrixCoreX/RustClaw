use std::path::{Path, PathBuf};

use super::{locator, trim_path_token};

pub(crate) fn dedup_and_sort_paths(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    paths.dedup();
}

pub(crate) fn resolve_existing_file_under_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let candidate = candidate_path_from_root(root, raw_path)?;
    let canonical = candidate.canonicalize().ok()?;
    canonical.is_file().then_some(canonical)
}

pub(crate) fn resolve_existing_dir_under_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let candidate = candidate_path_from_root(root, raw_path)?;
    let canonical = candidate.canonicalize().ok()?;
    canonical.is_dir().then_some(canonical)
}

fn candidate_path_from_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let cleaned = trim_path_token(raw_path);
    if cleaned.is_empty() {
        return None;
    }
    if locator::has_windows_drive_prefix(&cleaned) {
        return Some(PathBuf::from(cleaned));
    }
    let relative = cleaned
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_string();
    if relative.is_empty() {
        return Some(root.to_path_buf());
    }
    Some(root.join(relative))
}
