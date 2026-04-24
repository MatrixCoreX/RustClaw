use serde_json::Value;
use std::path::Path;

use super::LoopState;

pub(super) fn rewrite_run_cmd_with_written_aliases(
    command: &str,
    loop_state: &LoopState,
) -> String {
    if loop_state.written_file_aliases.is_empty() {
        return command.to_string();
    }
    let mut rewritten = command.to_string();
    for token in command.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| matches!(c, '"' | '\''));
        if trimmed.is_empty() {
            continue;
        }
        for (alias, effective) in &loop_state.written_file_aliases {
            if trimmed == alias || trimmed == alias.trim_start_matches("./") {
                rewritten = rewritten.replace(token, &token.replace(trimmed, effective));
                break;
            }
        }
    }
    rewritten
}

pub(super) fn rewrite_tool_path_with_written_aliases(
    tool: &str,
    args: &mut Value,
    loop_state: &LoopState,
) {
    if !matches!(tool, "read_file" | "remove_file") || loop_state.written_file_aliases.is_empty() {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    let Some(path) = obj.get("path").and_then(|v| v.as_str()) else {
        return;
    };
    let normalized = path.trim().trim_start_matches("./");
    let Some(effective) = loop_state.written_file_aliases.get(normalized) else {
        return;
    };
    obj.insert("path".to_string(), Value::String(effective.clone()));
}

fn rewrite_path_field(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    match obj.get("path").and_then(|v| v.as_str()) {
        Some(current) if current == auto_locator_path => false,
        Some(current) => {
            // AUTO_LOCATOR 只是 turn 级"默认 path 兜底"，仅当 LLM 给的 path 看起来是
            // 猜测/无效（不是已存在的文件/目录）时才能覆盖。已显式且存在的具体路径
            // （例如 plan 中的 read_file(README.md) 与 read_file(service_notes.md) 这种
            // 多目标 read 链路）必须保留 LLM 原值，否则会把多步 read 全部 rewrite 成同一个
            // auto_locator path，导致下游 chat 拿到重复内容、看不到第二个目标的真实输出。
            let trimmed = current.trim();
            if !trimmed.is_empty() && Path::new(trimmed).exists() {
                return false;
            }
            obj.insert(
                "path".to_string(),
                Value::String(auto_locator_path.to_string()),
            );
            true
        }
        None => false,
    }
}

fn rewrite_root_field(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    match obj.get("root").and_then(|v| v.as_str()) {
        Some(current) if current == auto_locator_path => false,
        Some(current) => {
            // 与 rewrite_path_field 同义：已存在的真实 root（如 LLM 显式给的 fs_search.find_path
            // root="/home/.../docs"）不该被 turn 级 AUTO_LOCATOR 默认值覆盖。
            let trimmed = current.trim();
            if !trimmed.is_empty() && Path::new(trimmed).exists() {
                return false;
            }
            obj.insert(
                "root".to_string(),
                Value::String(auto_locator_path.to_string()),
            );
            true
        }
        None => false,
    }
}

pub(super) fn rewrite_args_with_auto_locator_path(
    normalized_skill: &str,
    args: &mut Value,
    loop_state: &LoopState,
) -> bool {
    let Some(auto_locator_path) = loop_state
        .output_vars
        .get("auto_locator_path")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return false;
    };
    let auto_path = Path::new(auto_locator_path);
    match normalized_skill {
        "read_file" if auto_path.is_file() => rewrite_path_field(args, auto_locator_path),
        "list_dir" if auto_path.is_dir() => rewrite_path_field(args, auto_locator_path),
        "system_basic" => {
            let action = args
                .as_object()
                .and_then(|obj| obj.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match action {
                "extract_field" | "extract_fields" | "structured_keys" | "read_range"
                    if auto_path.is_file() =>
                {
                    rewrite_path_field(args, auto_locator_path)
                }
                "inventory_dir" | "count_inventory" | "workspace_glance" | "tree_summary"
                    if auto_path.is_dir() =>
                {
                    rewrite_path_field(args, auto_locator_path)
                }
                "find_path" if auto_path.is_dir() => rewrite_root_field(args, auto_locator_path),
                _ => false,
            }
        }
        _ => false,
    }
}

fn replace_double_brace_placeholders(
    input: &str,
    vars: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = input.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn single_brace_key(input: &str) -> Option<&str> {
    if !(input.starts_with('{') && input.ends_with('}')) {
        return None;
    }
    let inner = &input[1..input.len().saturating_sub(1)];
    if inner.is_empty() || inner.contains('{') || inner.contains('}') {
        return None;
    }
    Some(inner)
}

fn angle_bracket_key(input: &str) -> Option<&str> {
    if !(input.starts_with('<') && input.ends_with('>')) {
        return None;
    }
    let inner = &input[1..input.len().saturating_sub(1)];
    if inner.is_empty() || inner.contains('<') || inner.contains('>') {
        return None;
    }
    Some(inner)
}

pub(super) fn resolve_arg_string(input: &str, loop_state: &LoopState) -> String {
    let replaced = replace_double_brace_placeholders(input, &loop_state.output_vars);
    if let Some(key) = single_brace_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
    }
    if let Some(key) = angle_bracket_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
        let normalized_key = key.trim().to_ascii_lowercase();
        if let Some(v) = loop_state.output_vars.get(&normalized_key) {
            return v.clone();
        }
    }
    replaced
}

pub(super) fn resolve_arg_value(value: &Value, loop_state: &LoopState) -> Value {
    match value {
        Value::String(s) => Value::String(resolve_arg_string(s, loop_state)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_arg_value(v, loop_state))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_arg_value(v, loop_state));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_args_with_auto_locator_path;
    use crate::agent_engine::LoopState;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_arg_resolver_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn auto_locator_rewrites_system_basic_file_path() {
        let root = TempDirGuard::new("readme_file");
        let readme = root.path.join("README.md");
        fs::write(&readme, "# title\n").expect("write readme");
        let readme_path = readme.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), readme_path.clone());
        let mut args = json!({
            "action": "read_range",
            "path": "/tmp/README",
            "mode": "head",
            "n": 20
        });
        assert!(rewrite_args_with_auto_locator_path(
            "system_basic",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(readme_path.as_str())
        );
    }

    #[test]
    fn auto_locator_rewrites_directory_root_for_find_path() {
        let root = TempDirGuard::new("workspace_dir");
        let document = root.path.join("document");
        fs::create_dir_all(&document).expect("create document");
        let document_path = document.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), document_path.clone());
        // 用一个明确不存在的 root，以贴近 AUTO_LOCATOR 的"兜底猜测路径"语义。
        let mut args = json!({
            "action": "find_path",
            "root": "/nonexistent_root_for_auto_locator_test_xyz",
            "name": "manual_note.txt"
        });
        assert!(rewrite_args_with_auto_locator_path(
            "system_basic",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some(document_path.as_str())
        );
    }

    #[test]
    fn auto_locator_preserves_explicit_existing_path() {
        // F8 回归用例：当 LLM 显式给的 path 是真实存在的具体文件时（典型场景：
        // 多文件 read 链路第二个 read_file），AUTO_LOCATOR 不得覆盖它。
        let root = TempDirGuard::new("explicit_existing");
        let readme = root.path.join("README.md");
        let notes = root.path.join("notes.md");
        fs::write(&readme, "# readme\n").expect("write readme");
        fs::write(&notes, "# notes\n").expect("write notes");
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), notes.display().to_string());
        let mut args = json!({"path": readme.display().to_string()});
        let rewritten = rewrite_args_with_auto_locator_path("read_file", &mut args, &loop_state);
        assert!(!rewritten, "explicit existing path must not be rewritten");
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(readme.display().to_string().as_str())
        );
    }
}
