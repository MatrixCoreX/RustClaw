use super::{storage_path_for, KbRuntime};
use anyhow::{anyhow, Context, Result};
use rustclaw_skill_sdk::ExpectedPathKind;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(super) struct ScanTarget {
    pub(super) root: PathBuf,
    pub(super) is_file: bool,
    pub(super) storage_prefix: String,
}

#[derive(Debug, Default)]
pub(super) struct ScanManifest {
    pub(super) files: Vec<PathBuf>,
    pub(super) warnings: Vec<String>,
}

pub(super) fn build_scan_targets(
    runtime: &KbRuntime,
    raw_paths: &[String],
) -> Result<Vec<ScanTarget>> {
    let mut out = Vec::new();
    for raw in raw_paths {
        let canonical = runtime
            .path_policy
            .resolve_existing(raw, ExpectedPathKind::Any)
            .map_err(|error| anyhow!("{}: {}", error.code, error.detail))?;
        let meta = fs::metadata(&canonical)
            .with_context(|| format!("stat failed: {}", canonical.display()))?;
        let storage_prefix = storage_path_for(&canonical, &runtime.workspace_root);
        out.push(ScanTarget {
            root: canonical,
            is_file: meta.is_file(),
            storage_prefix,
        });
    }
    Ok(out)
}

pub(super) fn collect_target_files(
    targets: &[ScanTarget],
    max_depth: usize,
) -> Result<ScanManifest> {
    let mut seen_files = HashSet::new();
    let mut visited_dirs = HashSet::new();
    let mut manifest = ScanManifest::default();
    for target in targets {
        collect_files(
            &target.root,
            0,
            max_depth,
            &mut manifest,
            &mut seen_files,
            &mut visited_dirs,
        )?;
    }
    manifest.files.sort();
    manifest.warnings.sort();
    Ok(manifest)
}

fn collect_files(
    path: &Path,
    depth: usize,
    max_depth: usize,
    manifest: &mut ScanManifest,
    seen_files: &mut HashSet<PathBuf>,
    visited_dirs: &mut HashSet<PathBuf>,
) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("path not found: {}", path.display()));
    }
    if path.is_file() {
        let canonical = fs::canonicalize(path)
            .with_context(|| format!("canonicalize failed: {}", path.display()))?;
        if seen_files.insert(canonical.clone()) {
            manifest.files.push(canonical);
        }
        return Ok(());
    }
    let canonical_dir = fs::canonicalize(path)
        .with_context(|| format!("canonicalize failed: {}", path.display()))?;
    if !visited_dirs.insert(canonical_dir) {
        return Ok(());
    }
    if depth >= max_depth {
        manifest.warnings.push(format!(
            "directory depth budget reached at {} (max_depth={max_depth})",
            path.display()
        ));
        return Ok(());
    }
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("read_dir failed: {}", path.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let child = entry.path();
        if child.is_dir() {
            collect_files(
                &child,
                depth + 1,
                max_depth,
                manifest,
                seen_files,
                visited_dirs,
            )?;
        } else if child.is_file() {
            collect_files(
                &child,
                depth + 1,
                max_depth,
                manifest,
                seen_files,
                visited_dirs,
            )?;
        }
    }
    Ok(())
}

pub(super) fn path_matches_any_scan_target(path: &Path, targets: &[ScanTarget]) -> bool {
    let stored = super::normalize_path_string(path);
    targets.iter().any(|target| {
        let absolute_match = if target.is_file {
            path == target.root
        } else {
            path.starts_with(&target.root)
        };
        if absolute_match {
            return true;
        }
        if target.is_file {
            return stored == target.storage_prefix;
        }
        if target.storage_prefix.is_empty() {
            return !path.is_absolute();
        }
        stored == target.storage_prefix
            || stored.starts_with(&format!("{}/", target.storage_prefix))
    })
}
