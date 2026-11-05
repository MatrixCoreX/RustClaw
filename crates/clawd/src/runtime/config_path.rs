use std::path::{Path, PathBuf};

use super::state::AppState;

pub(crate) fn active_runtime_config_path(state: &AppState) -> PathBuf {
    let configured = state.reload_ctx.config_path_for_reload.trim();
    if configured.is_empty() {
        return state.skill_rt.workspace_root.join("configs/config.toml");
    }
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    }
}
