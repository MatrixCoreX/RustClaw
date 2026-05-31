use std::path::Path;

use serde_json::Value;

pub(crate) fn ensure_default_output_dir_for_skill_args(
    workspace_root: &Path,
    skill_name: &str,
    args: Value,
) -> Value {
    let Some(mut obj) = args.as_object().cloned() else {
        return args;
    };
    match skill_name {
        "image_generate" | "image_edit" => {
            let has_output_path = obj
                .get("output_path")
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty());
            if has_output_path {
                return Value::Object(obj);
            }
            let section = if skill_name == "image_edit" {
                "image_edit"
            } else {
                "image_generation"
            };
            let dir = resolve_output_dir_from_config(workspace_root, section);
            let ts = crate::now_ts_u64();
            let prefix = if skill_name == "image_edit" {
                "edit"
            } else {
                "gen"
            };
            let suggested = format!("{dir}/{prefix}-{ts}.png");
            obj.insert("output_path".to_string(), Value::String(suggested));
            Value::Object(obj)
        }
        _ => Value::Object(obj),
    }
}

fn resolve_output_dir_from_config(workspace_root: &Path, section: &str) -> String {
    let cfg_path = workspace_root.join("configs/config.toml");
    let raw = match std::fs::read_to_string(cfg_path) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    value
        .get(section)
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("default_output_dir"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("document")
        .to_string()
}

#[cfg(test)]
#[path = "output_dirs_tests.rs"]
mod tests;
