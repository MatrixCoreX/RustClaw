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
            if !trimmed.is_empty()
                && missing_file_name_conflicts_with_auto_locator(trimmed, auto_locator_path)
            {
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

fn missing_file_name_conflicts_with_auto_locator(current: &str, auto_locator_path: &str) -> bool {
    let current_path = Path::new(current);
    let auto_path = Path::new(auto_locator_path);
    if current_path.exists() || !auto_path.is_file() {
        return false;
    }
    let Some(current_name) = current_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(normalize_file_name_token)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if !current_name.contains('.') {
        return false;
    }
    let auto_name = auto_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(normalize_file_name_token)
        .unwrap_or_default();
    let auto_stem = auto_path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(normalize_file_name_token)
        .unwrap_or_default();
    if auto_name.is_empty() {
        return false;
    }
    !(current_name == auto_name
        || (!auto_stem.is_empty() && current_name.contains(auto_stem.as_str())))
}

fn normalize_file_name_token(value: &str) -> String {
    value.trim().replace('-', "_").to_ascii_lowercase()
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
        "audio_synthesize" => normalize_audio_synthesize_arg_aliases(args),
        "browser_web" => normalize_browser_web_arg_aliases(args),
        "config_edit" => normalize_config_edit_arg_aliases(args),
        "fs_search" => normalize_fs_search_arg_aliases(args),
        "image_generate" => normalize_image_generate_arg_aliases(args),
        "image_edit" => normalize_image_edit_arg_aliases(args),
        "kb" => normalize_kb_arg_aliases(args),
        "music_generate" => normalize_music_generate_arg_aliases(args),
        "run_cmd" => normalize_run_cmd_arg_aliases(args),
        "service_control" => normalize_service_control_arg_aliases(args),
        "video_generate" => normalize_video_generate_arg_aliases(args),
        _ => false,
    }
}

fn normalize_run_cmd_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    move_string_alias_if_missing(obj, "command", &["cmd", "shell_command"])
}

fn normalize_config_edit_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    move_value_alias_if_missing(obj, "value", &["new_value", "target_value"])
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

fn normalize_image_edit_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    move_string_alias_if_missing(obj, "instruction", &["prompt", "query", "text"])
}

fn normalize_image_generate_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    changed |= move_string_alias_if_missing(
        obj,
        "prompt",
        &["subject", "description", "instruction", "text", "query"],
    );
    changed |= move_string_alias_if_missing(obj, "size", &["resolution", "dimensions"]);
    changed |= move_size_from_width_height_if_missing(obj);
    changed
}

fn normalize_audio_synthesize_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    move_string_alias_if_missing(obj, "text", &["input", "prompt", "content"])
}

fn normalize_browser_web_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let mut changed = false;
    if matches!(action.as_str(), "search_page" | "search_extract") {
        changed |= move_value_alias_if_missing(obj, "top_k", &["max_results"]);
        changed |= normalize_integer_range_field(obj, "top_k", 1, 20);
    }
    if action == "search_extract" {
        changed |= move_value_alias_if_missing(obj, "extract_top_n", &["max_pages"]);
        changed |= normalize_integer_range_field(obj, "extract_top_n", 1, 10);
    }
    if action == "open_extract" {
        changed |= normalize_integer_range_field(obj, "max_pages", 1, 10);
    }
    if matches!(action.as_str(), "open_extract" | "search_extract") {
        changed |= normalize_integer_range_field(obj, "max_text_chars", 100, 200_000);
        changed |= normalize_integer_range_field(obj, "min_content_chars", 20, 10_000);
        changed |= normalize_browser_web_content_mode(obj);
        changed |= normalize_browser_web_wait_until(obj);
    }
    changed
}

fn normalize_browser_web_content_mode(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(value) = obj.get("content_mode").and_then(Value::as_str) else {
        return false;
    };
    let token = value.trim().to_ascii_lowercase().replace('-', "_");
    let normalized = match token.as_str() {
        "clean" => "clean",
        "raw" | "html" | "source" => "raw",
        _ => "clean",
    };
    if value == normalized {
        return false;
    }
    obj.insert(
        "content_mode".to_string(),
        Value::String(normalized.to_string()),
    );
    true
}

fn normalize_browser_web_wait_until(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(value) = obj.get("wait_until").and_then(Value::as_str) else {
        return false;
    };
    let token = value.trim().to_ascii_lowercase().replace(['-', '_'], "");
    let normalized = match token.as_str() {
        "domcontentloaded" => "domcontentloaded",
        "load" | "loaded" | "pageload" => "load",
        "networkidle" | "networkidled" | "idle" => "networkidle",
        _ => "domcontentloaded",
    };
    if value == normalized {
        return false;
    }
    obj.insert(
        "wait_until".to_string(),
        Value::String(normalized.to_string()),
    );
    true
}

fn normalize_integer_range_field(
    obj: &mut serde_json::Map<String, Value>,
    field: &str,
    min: i64,
    max: i64,
) -> bool {
    let Some(value) = obj.get(field).and_then(integer_value) else {
        return false;
    };
    let normalized = value.clamp(min, max);
    if normalized == value {
        return false;
    }
    obj.insert(
        field.to_string(),
        Value::Number(serde_json::Number::from(normalized)),
    );
    true
}

fn integer_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn normalize_video_generate_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    move_string_alias_if_missing(
        obj,
        "prompt",
        &["subject", "description", "instruction", "text", "query"],
    )
}

fn normalize_music_generate_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    move_string_alias_if_missing(obj, "prompt", &["description", "subject", "theme", "text"])
}

fn normalize_service_control_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    changed |= move_string_alias_if_missing(obj, "target", &["unit", "name"]);
    changed |= move_string_alias_if_missing(obj, "manager_type", &["manager"]);
    changed
}

fn move_size_from_width_height_if_missing(obj: &mut serde_json::Map<String, Value>) -> bool {
    if obj
        .get("size")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    let Some(width) = dimension_value_to_string(obj.get("width")) else {
        return false;
    };
    let Some(height) = dimension_value_to_string(obj.get("height")) else {
        return false;
    };
    obj.insert(
        "size".to_string(),
        Value::String(format!("{width}x{height}")),
    );
    true
}

fn dimension_value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => number
            .as_u64()
            .filter(|value| *value > 0)
            .map(|value| value.to_string()),
        Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed != "0" {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
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
    changed |= move_value_alias_if_missing(obj, "max_results", &["limit", "max_entries"]);
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

fn normalize_kb_arg_aliases(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let Some(is_ingest) = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.eq_ignore_ascii_case("ingest"))
    else {
        return false;
    };
    let mut changed = move_string_alias_if_missing(
        obj,
        "namespace",
        &["kb_name", "kb_namespace", "knowledge_base_name"],
    );
    if !is_ingest || kb_ingest_has_source_paths(obj) {
        return changed;
    }
    if let Some(path) = ["source", "source_path", "file_path"]
        .iter()
        .find_map(|alias| {
            obj.get(*alias)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
    {
        obj.insert("paths".to_string(), Value::Array(vec![Value::String(path)]));
        changed = true;
    } else if let Some(paths) = ["sources", "source_paths", "file_paths"]
        .iter()
        .find_map(|alias| {
            let values = obj.get(*alias)?.as_array()?;
            let paths = values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_string()))
                .collect::<Vec<_>>();
            (!paths.is_empty()).then_some(paths)
        })
    {
        obj.insert("paths".to_string(), Value::Array(paths));
        changed = true;
    }
    changed
}

fn kb_ingest_has_source_paths(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("paths").is_some_and(|value| match value {
        Value::Array(items) => items
            .iter()
            .any(|item| item.as_str().is_some_and(|path| !path.trim().is_empty())),
        Value::String(path) => !path.trim().is_empty(),
        _ => false,
    }) || obj
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty())
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
        "config_basic" => {
            let action = args
                .as_object()
                .and_then(|obj| obj.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match action {
                "read_field" | "read_fields" if auto_path.is_file() => {
                    rewrite_path_field(args, auto_locator_path, allow_missing_rewrite)
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

fn path_arg_key_allows_listing_placeholder(key: Option<&str>) -> bool {
    let Some(key) = key else {
        return false;
    };
    matches!(
        key,
        "path" | "paths" | "file_path" | "file_paths" | "source_path" | "target_path"
    )
}

fn listing_placeholder_index(input: &str) -> Option<usize> {
    let trimmed = input.trim();
    let key = if let Some(key) = angle_bracket_key(trimmed) {
        key
    } else {
        let tail = trimmed
            .rsplit_once('/')
            .map(|(_, tail)| tail)
            .or_else(|| trimmed.rsplit_once('\\').map(|(_, tail)| tail))?;
        angle_bracket_key(tail)?
    };
    let normalized = key.trim().to_ascii_lowercase();
    let digits = normalized
        .strip_prefix("file")
        .or_else(|| normalized.strip_prefix("path"))
        .or_else(|| normalized.strip_prefix("entry"))?;
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let parsed = digits.parse::<usize>().ok()?;
    Some(parsed.saturating_sub(1))
}

fn resolve_listing_placeholder_path(input: &str, loop_state: &LoopState) -> Option<String> {
    let index = listing_placeholder_index(input)?;
    latest_structured_listing_paths(loop_state)
        .get(index)
        .cloned()
}

fn latest_structured_listing_paths(loop_state: &LoopState) -> Vec<String> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let Some(paths) = structured_listing_paths_from_output(output) else {
            continue;
        };
        if !paths.is_empty() {
            return paths;
        }
    }
    Vec::new()
}

fn structured_listing_paths_from_output(output: &str) -> Option<Vec<String>> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    structured_listing_paths_from_value(&value).or_else(|| {
        ["extra", "data", "result"].iter().find_map(|key| {
            value
                .get(*key)
                .and_then(structured_listing_paths_from_value)
        })
    })
}

fn structured_listing_paths_from_value(value: &Value) -> Option<Vec<String>> {
    if !looks_like_structured_listing(value) {
        return None;
    }
    let base = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let mut paths = Vec::new();
    if let Some(entries) = value.get("entries").and_then(Value::as_array) {
        for entry in entries {
            if let Some(path) = entry_listing_path(entry, base) {
                paths.push(path);
            }
        }
    }
    if paths.is_empty() {
        if let Some(names) = value.get("names").and_then(Value::as_array) {
            paths.extend(
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| join_listing_base(base, name)),
            );
        }
    }
    (!paths.is_empty()).then_some(paths)
}

fn looks_like_structured_listing(value: &Value) -> bool {
    let action_matches = value
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action| {
            matches!(
                action,
                "inventory_dir" | "list_dir" | "list_names" | "list_entries"
            )
        });
    action_matches || value.get("entries").is_some() || value.get("names").is_some()
}

fn entry_listing_path(entry: &Value, base: Option<&str>) -> Option<String> {
    for key in ["path", "resolved_path", "relative_path"] {
        if let Some(path) = entry
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            if Path::new(path).is_absolute() || path.contains('/') || path.contains('\\') {
                return Some(path.to_string());
            }
            return Some(join_listing_base(base, path));
        }
    }
    entry
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| join_listing_base(base, name))
}

fn join_listing_base(base: Option<&str>, name: &str) -> String {
    if Path::new(name).is_absolute() {
        return name.to_string();
    }
    let Some(base) = base else {
        return name.to_string();
    };
    let base = base.trim().trim_end_matches(['/', '\\']);
    if base.is_empty() {
        return name.to_string();
    }
    format!("{base}/{}", name.trim_start_matches(['/', '\\']))
}

pub(super) fn resolve_arg_value(value: &Value, loop_state: &LoopState) -> Value {
    resolve_arg_value_with_key(value, None, loop_state)
}

fn resolve_arg_value_with_key(value: &Value, key: Option<&str>, loop_state: &LoopState) -> Value {
    match value {
        Value::String(s) => {
            let resolved = resolve_arg_string(s, loop_state);
            let resolved = if path_arg_key_allows_listing_placeholder(key) {
                resolve_listing_placeholder_path(&resolved, loop_state).unwrap_or(resolved)
            } else {
                resolved
            };
            Value::String(resolved)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_arg_value_with_key(v, key, loop_state))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(
                    k.clone(),
                    resolve_arg_value_with_key(v, Some(k), loop_state),
                );
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
#[path = "arg_resolver_tests.rs"]
mod tests;
