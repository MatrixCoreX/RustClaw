use std::path::Path;

use toml::Value as TomlValue;

pub(crate) fn ensure_default_file_path(workspace_root: &Path, input: &str) -> String {
    let default_dir = resolve_file_default_output_dir_from_config(workspace_root);
    let p = input.trim();
    if p.is_empty() {
        return format!("{default_dir}/artifact-{}.txt", crate::now_ts_u64());
    }
    if Path::new(p).is_absolute()
        || p.contains('/')
        || p.contains('\\')
        || p.starts_with("./")
        || p.starts_with("../")
    {
        return p.to_string();
    }
    format!("{default_dir}/{p}")
}

fn resolve_file_default_output_dir_from_config(workspace_root: &Path) -> String {
    let cfg_path = workspace_root.join("configs/config.toml");
    let raw = match std::fs::read_to_string(cfg_path) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    let value: TomlValue = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    value
        .get("file_generation")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("default_output_dir"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("document")
        .to_string()
}
