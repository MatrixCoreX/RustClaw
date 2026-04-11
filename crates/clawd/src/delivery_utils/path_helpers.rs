use std::path::{Path, PathBuf};

use super::{locator, trim_path_token};

pub(crate) fn dedup_and_sort_paths(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    paths.dedup();
}

pub(crate) fn resolve_existing_file_under_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let canonical = resolve_existing_path_under_root_case_insensitive(root, raw_path)?;
    canonical.is_file().then_some(canonical)
}

pub(crate) fn resolve_existing_dir_under_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let canonical = resolve_existing_path_under_root_case_insensitive(root, raw_path)?;
    canonical.is_dir().then_some(canonical)
}

pub(crate) fn resolve_existing_path_under_root_case_insensitive(
    root: &Path,
    raw_path: &str,
) -> Option<PathBuf> {
    let candidate = candidate_path_from_root(root, raw_path)?;
    if let Ok(canonical) = candidate.canonicalize() {
        return Some(canonical);
    }
    resolve_case_insensitive_candidate_path(root, raw_path)
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

fn resolve_case_insensitive_candidate_path(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let cleaned = trim_path_token(raw_path);
    if cleaned.is_empty() || locator::has_windows_drive_prefix(&cleaned) {
        return None;
    }
    let relative = cleaned
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_string();
    let mut current = root.to_path_buf();
    if relative.is_empty() {
        return current.canonicalize().ok();
    }
    for segment in relative
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != ".")
    {
        if segment == ".." {
            current = current.parent()?.to_path_buf();
            continue;
        }
        let entries = std::fs::read_dir(&current).ok()?;
        let mut exact_match = None;
        let mut ci_matches = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == segment {
                exact_match = Some(entry.path());
                break;
            }
            if name.eq_ignore_ascii_case(segment) {
                ci_matches.push(entry.path());
            }
        }
        current = if let Some(path) = exact_match {
            path
        } else if ci_matches.len() == 1 {
            ci_matches.pop().unwrap_or_default()
        } else {
            return None;
        };
    }
    current.canonicalize().ok()
}
