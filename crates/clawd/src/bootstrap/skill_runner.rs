use std::path::{Path, PathBuf};

pub(crate) fn resolve_skill_runner_path(workspace_root: &Path) -> PathBuf {
    let explicit = claw_core::product_identity::env_string("SKILL_RUNNER_PATH").ok();
    let executable_path = std::env::current_exe().ok();
    resolve_skill_runner_path_from(
        workspace_root,
        explicit.as_deref(),
        executable_path.as_deref(),
    )
}

pub(crate) fn resolve_required_skill_runner_path(workspace_root: &Path) -> Result<PathBuf, String> {
    let path = resolve_skill_runner_path(workspace_root);
    validate_skill_runner_path(&path)?;
    Ok(path)
}

fn validate_skill_runner_path(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "required skill-runner binary is missing: path={}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = path
            .metadata()
            .map_err(|error| {
                format!(
                    "required skill-runner metadata is unavailable: path={} error={error}",
                    path.display()
                )
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!(
                "required skill-runner binary is not executable: path={}",
                path.display()
            ));
        }
    }
    Ok(())
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
