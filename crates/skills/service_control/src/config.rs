use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

const CONFIG_REL: &str = "configs/service_control.toml";
const DEFAULT_AMBIGUOUS_TARGETS: &[&str] = &["all", "*"];

#[derive(Debug, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    service_control: ServiceControlConfig,
}

#[derive(Debug, Deserialize, Default)]
struct ServiceControlConfig {
    #[serde(default)]
    ambiguous_target_aliases: Vec<String>,
}

pub(crate) fn is_ambiguous_target(target: &str) -> bool {
    let normalized = target.trim().to_lowercase();
    if normalized.is_empty() {
        return true;
    }
    ambiguous_target_aliases()
        .iter()
        .any(|alias| normalized == *alias || normalized.contains(alias))
}

fn ambiguous_target_aliases() -> &'static Vec<String> {
    static ALIASES: OnceLock<Vec<String>> = OnceLock::new();
    ALIASES.get_or_init(load_ambiguous_target_aliases)
}

fn load_ambiguous_target_aliases() -> Vec<String> {
    let mut aliases = DEFAULT_AMBIGUOUS_TARGETS
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    if let Some(root) = find_workspace_root() {
        let path = root.join(CONFIG_REL);
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(parsed) = toml::from_str::<RootConfig>(&raw) {
                aliases.extend(parsed.service_control.ambiguous_target_aliases);
            }
        }
    }

    aliases
        .into_iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("WORKSPACE_ROOT") {
        let path = PathBuf::from(raw.trim());
        if config_exists(path.as_path()) {
            return Some(path);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if config_exists(dir.as_path()) {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
}

fn config_exists(root: &Path) -> bool {
    root.join(CONFIG_REL).is_file()
}
