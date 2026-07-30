use std::path::Path;

use super::SkillsRegistry;

pub(super) fn load(base_path: &Path, overlay_dir: Option<&Path>) -> Result<SkillsRegistry, String> {
    let base_content = match std::fs::read_to_string(base_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "skill_registry_missing; path={}; repair=restore_the_configured_base_registry",
                base_path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "read base registry failed: {}: {error}",
                base_path.display()
            ));
        }
    };
    let base = SkillsRegistry::load_from_str_with_source(&base_content, base_path)?;
    let Some(overlay_dir) = overlay_dir else {
        return Ok(base);
    };
    if !overlay_dir.exists() {
        return Ok(base);
    }
    let mut paths = std::fs::read_dir(overlay_dir)
        .map_err(|error| {
            format!(
                "read overlay registry directory failed: {}: {error}",
                overlay_dir.display()
            )
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read overlay registry entry failed: {error}"))?;
    paths.sort();

    let mut merged = base_content;
    for path in paths {
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect overlay registry failed: {error}"))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("toml")
        {
            return Err(format!(
                "overlay registry contains an unsupported entry: {}",
                path.display()
            ));
        }
        let content = std::fs::read_to_string(&path).map_err(|error| {
            format!("read overlay registry failed: {}: {error}", path.display())
        })?;
        let fragment = SkillsRegistry::load_from_str_with_source(&content, &path)?;
        let names = fragment.all_names();
        if names.len() != 1 {
            return Err(format!(
                "overlay registry fragment must contain exactly one skill: {}",
                path.display()
            ));
        }
        let name = &names[0];
        if fragment.is_builtin(name) {
            return Err(format!(
                "overlay registry cannot declare builtin skill `{name}`: {}",
                path.display()
            ));
        }
        if base.is_known(name) {
            return Err(format!(
                "overlay registry cannot replace bundled skill `{name}`: {}",
                path.display()
            ));
        }
        if path.file_stem().and_then(|value| value.to_str()) != Some(name.as_str()) {
            return Err(format!(
                "overlay registry filename must match canonical skill `{name}`: {}",
                path.display()
            ));
        }
        if !merged.ends_with('\n') && !merged.is_empty() {
            merged.push('\n');
        }
        merged.push_str(&content);
        if !merged.ends_with('\n') {
            merged.push('\n');
        }
    }
    SkillsRegistry::load_from_str_with_source(&merged, base_path)
}
