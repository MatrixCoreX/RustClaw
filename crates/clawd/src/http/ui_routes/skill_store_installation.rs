#[derive(Debug, Clone)]
struct SkillStoreInstallSpec {
    package: String,
    binary: String,
}

fn runner_binary_name(raw_name: &str) -> Result<String, String> {
    let raw_name = raw_name.trim();
    if raw_name.is_empty()
        || raw_name.contains('/')
        || raw_name.contains('\\')
        || !raw_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err("invalid runner name in skill registry".to_string());
    }
    let normalized = raw_name.replace('_', "-");
    Ok(if normalized.ends_with("-skill") {
        normalized
    } else {
        format!("{normalized}-skill")
    })
}

fn skill_store_install_spec(
    state: &AppState,
    skill_name: &str,
) -> Result<Option<SkillStoreInstallSpec>, String> {
    let registry = state
        .get_skills_registry()
        .ok_or_else(|| "skills registry is not available".to_string())?;
    let entry = registry
        .get(skill_name)
        .ok_or_else(|| format!("unknown skill: {skill_name}"))?;
    if entry.kind != SkillKind::Runner {
        return Ok(None);
    }
    if entry.install_mode.as_deref() != Some("on_demand") {
        return Err(format!(
            "skill {skill_name} is not declared as an on-demand install"
        ));
    }
    let binary = runner_binary_name(&registry.runner_name(skill_name))?;
    let package = entry
        .install_package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(binary.as_str())
        .to_string();
    if !package
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err("invalid install package in skill registry".to_string());
    }
    Ok(Some(SkillStoreInstallSpec { package, binary }))
}

fn declared_skill_config_paths(
    state: &AppState,
    skill_name: &str,
) -> Result<Vec<PathBuf>, String> {
    let Some(registry) = state.get_skills_registry() else {
        return Ok(Vec::new());
    };
    let Some(entry) = registry.get(skill_name) else {
        return Ok(Vec::new());
    };
    entry
        .config_files
        .iter()
        .map(|relative| {
            let relative_path = Path::new(relative);
            let safe = !relative_path.is_absolute()
                && relative_path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
                && relative_path.starts_with("configs");
            if !safe {
                return Err(format!(
                    "unsafe config path for skill {skill_name}: {relative}"
                ));
            }
            Ok(state.skill_rt.workspace_root.join(relative_path))
        })
        .collect()
}

fn skill_config_state(state: &AppState, skill_name: &str) -> (Vec<String>, Vec<String>) {
    let Ok(paths) = declared_skill_config_paths(state, skill_name) else {
        return (Vec::new(), Vec::new());
    };
    let declared = paths
        .iter()
        .filter_map(|path| {
            path.strip_prefix(&state.skill_rt.workspace_root)
                .ok()
                .map(|relative| relative.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    let existing = paths
        .iter()
        .filter(|path| path.is_file())
        .filter_map(|path| {
            path.strip_prefix(&state.skill_rt.workspace_root)
                .ok()
                .map(|relative| relative.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    (declared, existing)
}

#[cfg(not(test))]
fn bounded_command_error(bytes: &[u8]) -> String {
    const MAX_CHARS: usize = 4_000;
    let text = String::from_utf8_lossy(bytes);
    let chars = text.chars().count();
    if chars <= MAX_CHARS {
        return text.into_owned();
    }
    text.chars().skip(chars - MAX_CHARS).collect()
}

#[cfg(not(test))]
async fn compile_skill_store_runner(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
) -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["build", "--release", "-p", spec.package.as_str()])
        .current_dir(&state.skill_rt.workspace_root)
        .env(
            "CARGO_TARGET_DIR",
            state.skill_rt.workspace_root.join("target"),
        )
        .output()
        .await
        .map_err(|error| format!("start skill build failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "skill build failed for {}: {}",
            spec.package,
            bounded_command_error(&output.stderr)
        ));
    }
    let binary_path = state
        .skill_rt
        .workspace_root
        .join("target/release")
        .join(&spec.binary);
    if !binary_path.is_file() {
        return Err(format!(
            "skill build completed without binary: {}",
            binary_path.display()
        ));
    }
    Ok(binary_path)
}

#[cfg(test)]
async fn compile_skill_store_runner(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
) -> Result<PathBuf, String> {
    let _package = &spec.package;
    let binary_path = state
        .skill_rt
        .workspace_root
        .join("target/release")
        .join(&spec.binary);
    if let Some(parent) = binary_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&binary_path, b"skill-store-test-binary").map_err(|error| error.to_string())?;
    Ok(binary_path)
}

fn remove_skill_store_binary(state: &AppState, spec: &SkillStoreInstallSpec) -> Result<bool, String> {
    let path = state
        .skill_rt
        .workspace_root
        .join("target/release")
        .join(&spec.binary);
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path)
        .map_err(|error| format!("remove skill binary {} failed: {error}", path.display()))?;
    Ok(true)
}

fn delete_declared_skill_configs(state: &AppState, skill_name: &str) -> Result<Vec<String>, String> {
    let mut deleted = Vec::new();
    for path in declared_skill_config_paths(state, skill_name)? {
        if !path.exists() {
            continue;
        }
        fs::remove_file(&path)
            .map_err(|error| format!("remove skill config {} failed: {error}", path.display()))?;
        if let Ok(relative) = path.strip_prefix(&state.skill_rt.workspace_root) {
            deleted.push(relative.to_string_lossy().into_owned());
        }
    }
    Ok(deleted)
}
