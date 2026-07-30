use std::path::{Path, PathBuf};

pub const WORKSPACE_STATE_DIR_NAME: &str = ".agent-runtime";

pub fn workspace_state_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join(WORKSPACE_STATE_DIR_NAME)
}

pub fn workspace_artifacts_root(workspace_root: &Path) -> PathBuf {
    workspace_state_root(workspace_root).join("artifacts")
}

pub fn known_workspace_state_roots(workspace_root: &Path) -> Vec<PathBuf> {
    vec![workspace_state_root(workspace_root)]
}

pub fn is_known_workspace_state_dir_name(name: &str) -> bool {
    name == WORKSPACE_STATE_DIR_NAME
}
