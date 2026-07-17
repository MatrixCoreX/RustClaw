use std::path::{Path, PathBuf};

const SKILL_RUNNER_PATH_ENV: &str = "RUSTCLAW_SKILL_RUNNER_PATH";

pub(crate) fn resolve_skill_runner_path(workspace_root: &Path) -> PathBuf {
    let explicit = std::env::var(SKILL_RUNNER_PATH_ENV).ok();
    let executable_path = std::env::current_exe().ok();
    resolve_skill_runner_path_from(
        workspace_root,
        explicit.as_deref(),
        executable_path.as_deref(),
    )
}

fn resolve_skill_runner_path_from(
    workspace_root: &Path,
    explicit: Option<&str>,
    executable_path: Option<&Path>,
) -> PathBuf {
    if let Some(path) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        let path = Path::new(path);
        return if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
    }

    let installed_companion = executable_path
        .and_then(Path::parent)
        .map(|parent| parent.join("skill-runner"));
    if let Some(path) = installed_companion.as_ref().filter(|path| path.is_file()) {
        return path.clone();
    }

    let workspace_candidate = workspace_root.join("target/release/skill-runner");
    if workspace_candidate.is_file() {
        return workspace_candidate;
    }

    installed_companion.unwrap_or(workspace_candidate)
}

#[cfg(test)]
#[path = "skill_runner_tests.rs"]
mod tests;
