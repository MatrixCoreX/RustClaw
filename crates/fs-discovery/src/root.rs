use std::path::{Component, Path, PathBuf};

use crate::DiscoveryError;

pub fn resolve_root(
    workspace_root: &Path,
    input: &Path,
) -> Result<(PathBuf, PathBuf), DiscoveryError> {
    if input
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(DiscoveryError::ParentTraversal);
    }
    let workspace = workspace_root
        .canonicalize()
        .map_err(|error| DiscoveryError::WorkspaceResolution(error.to_string()))?;
    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        workspace.join(input)
    };
    let candidate_metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|error| DiscoveryError::PathResolution(error.to_string()))?;
    if candidate_metadata.file_type().is_symlink() {
        return Err(DiscoveryError::UnsupportedRoot);
    }
    let root = candidate
        .canonicalize()
        .map_err(|error| DiscoveryError::PathResolution(error.to_string()))?;
    if !root.starts_with(&workspace) {
        return Err(DiscoveryError::OutsideWorkspace);
    }
    let metadata = std::fs::symlink_metadata(&root)
        .map_err(|error| DiscoveryError::PathResolution(error.to_string()))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(DiscoveryError::UnsupportedRoot);
    }
    Ok((workspace, root))
}

pub fn relative_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}
