use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;

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

fn broad_current_workspace_auto_locator(loop_state: &LoopState) -> bool {
    let Some(contract) = loop_state.output_contract.as_ref() else {
        return false;
    };
    contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && contract.locator_hint.trim().is_empty()
}

fn rewrite_path_field(
    args: &mut Value,
    auto_locator_path: &str,
    allow_missing_rewrite: bool,
) -> bool {
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
            if !trimmed.is_empty() && !allow_missing_rewrite {
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

fn rewrite_root_field(
    args: &mut Value,
    auto_locator_path: &str,
    allow_missing_rewrite: bool,
) -> bool {
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
            if !trimmed.is_empty() && !allow_missing_rewrite {
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

fn set_root_field_if_missing(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if obj.get("root").and_then(|v| v.as_str()).is_some() {
        return false;
    }
    obj.insert(
        "root".to_string(),
        Value::String(auto_locator_path.to_string()),
    );
    true
}

pub(super) fn normalize_skill_arg_aliases(normalized_skill: &str, args: &mut Value) -> bool {
    if crate::virtual_tools::normalize_virtual_tool_arg_aliases(normalized_skill, args) {
        return true;
    }
    match normalized_skill {
        "fs_search" => normalize_fs_search_arg_aliases(args),
        _ => false,
    }
}

fn move_string_alias_if_missing(
    obj: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) -> bool {
    if obj
        .get(canonical)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    let Some(value) = aliases.iter().find_map(|alias| {
        obj.get(*alias)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }) else {
        return false;
    };
    obj.insert(canonical.to_string(), Value::String(value));
    true
}

fn move_value_alias_if_missing(
    obj: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) -> bool {
    if obj.get(canonical).is_some() {
        return false;
    }
    let Some(value) = aliases.iter().find_map(|alias| obj.get(*alias).cloned()) else {
        return false;
    };
    obj.insert(canonical.to_string(), value);
    true
}

fn normalize_fs_search_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    changed |= move_string_alias_if_missing(
        obj,
        "root",
        &["search_root", "search_dir", "directory", "dir"],
    );
    changed |= move_string_alias_if_missing(obj, "pattern", &["name_pattern"]);
    changed |= move_value_alias_if_missing(obj, "max_results", &["limit"]);
    changed |= normalize_fs_search_find_path_contract(obj);
    changed |= normalize_fs_search_action_aliases(obj);
    if obj.get("action").and_then(|value| value.as_str()).is_none()
        && (obj
            .get("pattern")
            .and_then(|value| value.as_str())
            .is_some()
            || obj.get("name").and_then(|value| value.as_str()).is_some()
            || obj
                .get("keyword")
                .and_then(|value| value.as_str())
                .is_some())
    {
        obj.insert("action".to_string(), Value::String("find_name".to_string()));
        changed = true;
    }
    if obj
        .get("action")
        .and_then(|value| value.as_str())
        .is_some_and(|action| action.eq_ignore_ascii_case("find_name"))
    {
        changed |=
            move_string_alias_if_missing(obj, "pattern", &["name", "keyword", "query", "target"]);
        changed |= normalize_find_name_pattern_for_fs_search(obj);
    } else if obj
        .get("action")
        .and_then(|value| value.as_str())
        .is_some_and(|action| action.eq_ignore_ascii_case("grep_text"))
    {
        changed |= move_string_alias_if_missing(obj, "query", &["pattern", "keyword", "text"]);
    } else if obj
        .get("action")
        .and_then(|value| value.as_str())
        .is_some_and(|action| action.eq_ignore_ascii_case("find_ext"))
    {
        changed |= move_value_alias_if_missing(
            obj,
            "ext",
            &[
                "extension",
                "extensions",
                "ext_filter",
                "file_extension",
                "file_extensions",
            ],
        );
        changed |= move_string_alias_if_missing(obj, "pattern", &["name", "keyword", "query"]);
    }
    changed
}

fn normalize_fs_search_find_path_contract(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(action) = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if !action.eq_ignore_ascii_case("find_path") {
        return false;
    }
    let was_find_name = action.eq_ignore_ascii_case("find_name");
    let changed_root = move_string_alias_if_missing(obj, "root", &["path", "base_path"]);
    let changed_pattern = move_string_alias_if_missing(
        obj,
        "pattern",
        &["target", "name", "keyword", "query", "name_pattern"],
    );
    obj.insert("action".to_string(), Value::String("find_name".to_string()));
    for alias in [
        "path",
        "base_path",
        "target",
        "name",
        "keyword",
        "query",
        "name_pattern",
    ] {
        obj.remove(alias);
    }
    changed_root || changed_pattern || !was_find_name
}

fn normalize_fs_search_action_aliases(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(action) = obj
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let normalized = match action.to_ascii_lowercase().as_str() {
        "find_file" | "find_files" | "find_filename" | "find_filenames" | "search_name"
        | "search_names" | "search_filename" | "search_filenames" | "name_search"
        | "file_search" | "find_content" => "find_name",
        "grep" | "grep_content" | "search_text" | "text_search" | "search_content" => "grep_text",
        "find_extension" | "search_extension" | "extension_search" => "find_ext",
        "images" | "image_search" | "find_image" | "find_images" => "find_images",
        _ => return false,
    };
    if action == normalized {
        return false;
    }
    obj.insert("action".to_string(), Value::String(normalized.to_string()));
    true
}

fn normalize_find_name_pattern_for_fs_search(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(current) = obj
        .get("pattern")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(normalized) = find_name_contains_pattern_from_globish(current) else {
        return false;
    };
    if normalized == current {
        return false;
    }
    obj.insert("pattern".to_string(), Value::String(normalized));
    true
}

fn find_name_contains_pattern_from_globish(pattern: &str) -> Option<String> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    let stripped = trimmed.trim_matches('*').trim();
    if stripped.is_empty()
        || stripped == trimmed
        || stripped.contains('*')
        || stripped.contains('?')
        || stripped.contains('[')
        || stripped.contains(']')
    {
        return None;
    }
    Some(stripped.to_string())
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
    let allow_missing_rewrite = !broad_current_workspace_auto_locator(loop_state);
    match normalized_skill {
        "read_file" if auto_path.is_file() => {
            rewrite_path_field(args, auto_locator_path, allow_missing_rewrite)
        }
        "list_dir" if auto_path.is_dir() => {
            rewrite_path_field(args, auto_locator_path, allow_missing_rewrite)
        }
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
                    rewrite_path_field(args, auto_locator_path, allow_missing_rewrite)
                }
                "inventory_dir" | "count_inventory" | "workspace_glance" | "tree_summary"
                    if auto_path.is_dir() =>
                {
                    rewrite_path_field(args, auto_locator_path, allow_missing_rewrite)
                }
                "find_path" if auto_path.is_dir() => {
                    rewrite_root_field(args, auto_locator_path, allow_missing_rewrite)
                }
                _ => false,
            }
        }
        "fs_search" if auto_path.is_dir() => {
            rewrite_root_field(args, auto_locator_path, allow_missing_rewrite)
                || set_root_field_if_missing(args, auto_locator_path)
        }
        _ => false,
    }
}

fn replace_double_brace_placeholders(
    input: &str,
    vars: &std::collections::HashMap<String, String>,
) -> String {
    static PLACEHOLDER_RE: OnceLock<Regex> = OnceLock::new();
    let re = PLACEHOLDER_RE.get_or_init(|| {
        Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").expect("double brace placeholder regex")
    });
    re.replace_all(input, |caps: &regex::Captures<'_>| {
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let key = caps.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        vars.get(key).cloned().unwrap_or_else(|| whole.to_string())
    })
    .into_owned()
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
    use super::{
        normalize_skill_arg_aliases, resolve_arg_string, rewrite_args_with_auto_locator_path,
    };
    use crate::{agent_engine::LoopState, IntentOutputContract, OutputLocatorKind};
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
    fn resolve_arg_string_replaces_trimmed_double_brace_placeholders() {
        let mut loop_state = LoopState::new(1);
        loop_state
            .output_vars
            .insert("last_output[1]".to_string(), "clawd.log".to_string());
        loop_state
            .output_vars
            .insert("last_output.0".to_string(), "act_plan.log".to_string());

        assert_eq!(
            resolve_arg_string("/logs/{{ last_output[1] }}", &loop_state),
            "/logs/clawd.log"
        );
        assert_eq!(
            resolve_arg_string("/logs/{{last_output.0}}", &loop_state),
            "/logs/act_plan.log"
        );
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
    fn fs_search_aliases_normalize_to_supported_contract() {
        let mut args = json!({
            "name_pattern": "*abcd*",
            "search_root": "/tmp/stem_unique",
            "limit": 25,
            "match_mode": "substring"
        });

        assert!(normalize_skill_arg_aliases("fs_search", &mut args));
        assert_eq!(
            args.get("action").and_then(|v| v.as_str()),
            Some("find_name")
        );
        assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("abcd"));
        assert_eq!(args.get("max_results").and_then(|v| v.as_u64()), Some(25));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some("/tmp/stem_unique")
        );
    }

    #[test]
    fn fs_search_find_path_contract_normalizes_to_find_name() {
        let mut args = json!({
            "action": "find_path",
            "path": "/tmp/workspace",
            "target": "archive",
            "limit": 10
        });

        assert!(normalize_skill_arg_aliases("fs_search", &mut args));
        assert_eq!(
            args.get("action").and_then(|v| v.as_str()),
            Some("find_name")
        );
        assert_eq!(
            args.get("pattern").and_then(|v| v.as_str()),
            Some("archive")
        );
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some("/tmp/workspace")
        );
        assert!(args.get("path").is_none());
        assert!(args.get("target").is_none());
    }

    #[test]
    fn fs_search_globish_find_name_pattern_normalizes_to_contains_pattern() {
        let mut args = json!({
            "action": "find_name",
            "pattern": "*report.md*",
            "directory": "/tmp/docs"
        });

        assert!(normalize_skill_arg_aliases("fs_search", &mut args));
        assert_eq!(
            args.get("pattern").and_then(|v| v.as_str()),
            Some("report.md")
        );
        assert_eq!(args.get("root").and_then(|v| v.as_str()), Some("/tmp/docs"));
    }

    #[test]
    fn fs_search_find_content_alias_normalizes_to_name_search_contract() {
        let mut args = json!({
            "action": "find_content",
            "query": "abcd",
            "dir": "/tmp/stem_unique"
        });

        assert!(normalize_skill_arg_aliases("fs_search", &mut args));
        assert_eq!(
            args.get("action").and_then(|v| v.as_str()),
            Some("find_name")
        );
        assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("abcd"));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some("/tmp/stem_unique")
        );
    }

    #[test]
    fn fs_search_find_ext_aliases_preserve_multi_extension_contract() {
        let mut args = json!({
            "action": "search_extension",
            "extensions": ["md", "txt"],
            "query": "log",
            "directory": "/tmp/docs"
        });

        assert!(normalize_skill_arg_aliases("fs_search", &mut args));
        assert_eq!(
            args.get("action").and_then(|v| v.as_str()),
            Some("find_ext")
        );
        assert_eq!(
            args.get("ext").and_then(|v| v.as_array()).map(Vec::len),
            Some(2)
        );
        assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("log"));
        assert_eq!(args.get("root").and_then(|v| v.as_str()), Some("/tmp/docs"));
    }

    #[test]
    fn auto_locator_sets_missing_fs_search_root() {
        let root = TempDirGuard::new("fs_search_auto_root");
        let search_root = root.path.join("stem_unique");
        fs::create_dir_all(&search_root).expect("create search root");
        let search_root_path = search_root.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), search_root_path.clone());
        loop_state.output_contract = Some(IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: search_root_path.clone(),
            ..IntentOutputContract::default()
        });
        let mut args = json!({
            "action": "find_name",
            "pattern": "abcd"
        });

        assert!(rewrite_args_with_auto_locator_path(
            "fs_search",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some(search_root_path.as_str())
        );
    }

    #[test]
    fn auto_locator_overwrites_missing_fs_search_root() {
        let root = TempDirGuard::new("fs_search_missing_root");
        let search_root = root.path.join("case_only");
        fs::create_dir_all(&search_root).expect("create search root");
        let search_root_path = search_root.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), search_root_path.clone());
        loop_state.output_contract = Some(IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: search_root_path.clone(),
            ..IntentOutputContract::default()
        });
        let mut args = json!({
            "action": "find_name",
            "pattern": "report.md",
            "root": "/nonexistent_case_only"
        });

        assert!(rewrite_args_with_auto_locator_path(
            "fs_search",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some(search_root_path.as_str())
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

    #[test]
    fn broad_current_workspace_auto_locator_does_not_overwrite_missing_inventory_path() {
        let root = TempDirGuard::new("broad_current_workspace");
        let root_path = root.path.display().to_string();
        let explicit_missing = root.path.join("archive").display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), root_path);
        loop_state.output_contract = Some(IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: String::new(),
            ..IntentOutputContract::default()
        });
        let mut args = json!({
            "action": "inventory_dir",
            "path": explicit_missing,
            "depth": 1
        });

        let rewritten = rewrite_args_with_auto_locator_path("system_basic", &mut args, &loop_state);

        assert!(
            !rewritten,
            "broad workspace fallback must not silently replace a concrete missing path"
        );
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(explicit_missing.as_str())
        );
    }

    #[test]
    fn concrete_auto_locator_still_overwrites_missing_inventory_path() {
        let root = TempDirGuard::new("concrete_locator");
        let archive = root.path.join("docs_archive");
        fs::create_dir_all(&archive).expect("create archive");
        let archive_path = archive.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), archive_path.clone());
        loop_state.output_contract = Some(IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: archive_path.clone(),
            ..IntentOutputContract::default()
        });
        let mut args = json!({
            "action": "inventory_dir",
            "path": "/nonexistent_dir_for_concrete_auto_locator_test_xyz",
            "depth": 1
        });

        let rewritten = rewrite_args_with_auto_locator_path("system_basic", &mut args, &loop_state);

        assert!(
            rewritten,
            "concrete locator should repair guessed missing paths"
        );
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(archive_path.as_str())
        );
    }
}
