use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct VirtualToolRewrite {
    pub(crate) runtime_tool: String,
    pub(crate) runtime_args: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct VirtualToolCanonicalCall {
    pub(crate) tool: String,
    pub(crate) args: Value,
}

pub(crate) fn canonicalize_legacy_tool_call(
    tool: &str,
    args: Value,
) -> Option<VirtualToolCanonicalCall> {
    match tool {
        "system_basic" => canonicalize_system_basic_call(args),
        "fs_search" => canonicalize_fs_search_call(args),
        "config_guard" => canonicalize_config_guard_call(args),
        "read_file" => canonicalize_standalone_fs_call("fs_basic", "read_text_range", args),
        "list_dir" => canonicalize_standalone_fs_call("fs_basic", "list_dir", args),
        "write_file" => canonicalize_standalone_fs_call("fs_basic", "write_text", args),
        "make_dir" => canonicalize_standalone_fs_call("fs_basic", "make_dir", args),
        "remove_file" => canonicalize_standalone_fs_call("fs_basic", "remove_path", args),
        _ => None,
    }
}

pub(crate) fn normalize_virtual_tool_arg_aliases(tool: &str, args: &mut Value) -> bool {
    match tool {
        "fs_basic" => normalize_fs_basic_args(args),
        "config_basic" => normalize_config_basic_args(args),
        _ => false,
    }
}

pub(crate) fn rewrite_virtual_tool_call(
    tool: &str,
    args: Value,
) -> Result<Option<VirtualToolRewrite>, String> {
    match tool {
        "fs_basic" => rewrite_fs_basic_call(args).map(Some),
        "config_basic" => rewrite_config_basic_call(args).map(Some),
        _ => Ok(None),
    }
}

fn canonicalize_system_basic_call(args: Value) -> Option<VirtualToolCanonicalCall> {
    let mut obj = args_object(args, "system_basic").ok()?;
    let action = action_name(&obj)?;
    let (tool, action) = match action.as_str() {
        "path_batch_facts" => ("fs_basic", "stat_paths"),
        "inventory_dir" => ("fs_basic", "list_dir"),
        "count_inventory" => ("fs_basic", "count_entries"),
        "read_range" => ("fs_basic", "read_text_range"),
        "compare_paths" => ("fs_basic", "compare_paths"),
        "find_path" => {
            move_value_alias_if_missing(&mut obj, "pattern", &["name"]);
            ("fs_basic", "find_entries")
        }
        "extract_field" => ("config_basic", "read_field"),
        "extract_fields" => ("config_basic", "read_fields"),
        "structured_keys" => ("config_basic", "list_keys"),
        "validate_structured" => ("config_basic", "validate"),
        _ => return None,
    };
    obj.insert("action".to_string(), Value::String(action.to_string()));
    Some(VirtualToolCanonicalCall {
        tool: tool.to_string(),
        args: Value::Object(obj),
    })
}

fn canonicalize_fs_search_call(args: Value) -> Option<VirtualToolCanonicalCall> {
    let mut obj = args_object(args, "fs_search").ok()?;
    let action = action_name(&obj)?;
    match action.as_str() {
        "find_name" | "find_path" => {
            move_value_alias_if_missing(
                &mut obj,
                "pattern",
                &[
                    "name",
                    "query",
                    "keyword",
                    "entry_name",
                    "entry_names",
                    "target",
                    "basename_pattern",
                    "name_pattern",
                ],
            );
            obj.insert(
                "action".to_string(),
                Value::String("find_entries".to_string()),
            );
            Some(VirtualToolCanonicalCall {
                tool: "fs_basic".to_string(),
                args: Value::Object(obj),
            })
        }
        "find_ext" => {
            move_value_alias_if_missing(
                &mut obj,
                "ext",
                &[
                    "extension",
                    "ext_filter",
                    "pattern",
                    "query",
                    "basename_pattern",
                ],
            );
            obj.insert(
                "action".to_string(),
                Value::String("find_entries".to_string()),
            );
            Some(VirtualToolCanonicalCall {
                tool: "fs_basic".to_string(),
                args: Value::Object(obj),
            })
        }
        "grep_text" => {
            move_value_alias_if_missing(&mut obj, "query", &["text", "keyword"]);
            promote_grep_pattern_to_query_if_missing(&mut obj);
            drop_redundant_grep_filename_filters(&mut obj);
            obj.insert("action".to_string(), Value::String("grep_text".to_string()));
            Some(VirtualToolCanonicalCall {
                tool: "fs_basic".to_string(),
                args: Value::Object(obj),
            })
        }
        _ => None,
    }
}

fn canonicalize_config_guard_call(args: Value) -> Option<VirtualToolCanonicalCall> {
    let mut obj = args_object(args, "config_guard").ok()?;
    obj.insert(
        "action".to_string(),
        Value::String("guard_config".to_string()),
    );
    obj.entry("path".to_string())
        .or_insert_with(|| Value::String("configs/config.toml".to_string()));
    Some(VirtualToolCanonicalCall {
        tool: "config_edit".to_string(),
        args: Value::Object(obj),
    })
}

fn canonicalize_standalone_fs_call(
    tool: &str,
    action: &str,
    args: Value,
) -> Option<VirtualToolCanonicalCall> {
    let mut obj = args_object(args, tool).ok()?;
    obj.insert("action".to_string(), Value::String(action.to_string()));
    Some(VirtualToolCanonicalCall {
        tool: tool.to_string(),
        args: Value::Object(obj),
    })
}

fn rewrite_fs_basic_call(args: Value) -> Result<VirtualToolRewrite, String> {
    let mut obj = args_object(args, "fs_basic")?;
    let action = action_name(&obj).unwrap_or_default();
    match action.as_str() {
        "stat_paths" => {
            if !obj.contains_key("paths") {
                move_value_alias_if_missing(&mut obj, "paths", &["path"]);
            }
            obj.insert(
                "action".to_string(),
                Value::String("path_batch_facts".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "list_dir" => {
            move_value_alias_if_missing(&mut obj, "path", &["root", "dir", "directory"]);
            move_value_alias_if_missing(&mut obj, "max_entries", &["limit"]);
            obj.insert(
                "action".to_string(),
                Value::String("inventory_dir".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "count_entries" => {
            move_value_alias_if_missing(&mut obj, "path", &["root", "dir", "directory"]);
            normalize_count_entries_filter_aliases(&mut obj);
            obj.insert(
                "action".to_string(),
                Value::String("count_inventory".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "read_text_range" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path"]);
            normalize_read_text_range_args(&mut obj);
            obj.insert("action".to_string(), Value::String("read_range".to_string()));
            Ok(rewrite_to("system_basic", obj))
        }
        "find_entries" => {
            let explicit_name_pattern = has_any_non_empty_arg(
                &obj,
                &["name_pattern", "basename_pattern", "filename_pattern"],
            );
            let explicit_exact_basename = explicit_find_entries_exact_basename_selector(&obj);
            move_value_alias_if_missing(&mut obj, "root", &["path", "dir", "directory"]);
            move_existing_directory_alias_to_root(
                &mut obj,
                &[
                    "target",
                    "target_path",
                    "base",
                    "base_path",
                    "search_path",
                    "search_dir",
                ],
            );
            move_value_alias_if_missing(
                &mut obj,
                "pattern",
                &[
                    "name",
                    "keyword",
                    "query",
                    "entry_name",
                    "entry_names",
                    "filename",
                    "name_pattern",
                    "basename_pattern",
                    "filename_pattern",
                    "glob",
                    "filter",
                    "file_filter",
                    "include",
                    "glob_pattern",
                    "include_pattern",
                    "match_pattern",
                ],
            );
            move_value_alias_if_missing(
                &mut obj,
                "ext",
                &[
                    "extension",
                    "extensions",
                    "ext_filter",
                    "file_extension",
                    "file_extensions",
                ],
            );
            move_value_alias_if_missing(&mut obj, "max_results", &["max_entries", "limit"]);
            if obj.get("max_results").is_some() {
                obj.remove("max_entries");
                obj.remove("limit");
            }
            promote_extension_names_to_ext(&mut obj);
            if explicit_name_pattern {
                promote_pure_extension_pattern_to_ext(&mut obj);
            } else {
                promote_globish_pattern_to_ext(&mut obj);
            }
            demote_existing_directory_pattern_to_root(&mut obj);
            let has_pattern = has_non_empty_arg(&obj, "pattern");
            let has_ext = has_non_empty_arg(&obj, "ext");
            if !has_pattern && !has_ext {
                if let Some(root) = obj.remove("root") {
                    obj.insert("path".to_string(), root);
                }
                if !obj.contains_key("max_entries") {
                    if let Some(max_results) = obj.remove("max_results") {
                        obj.insert("max_entries".to_string(), max_results);
                    }
                }
                if string_field_matches(&obj, &["target_kind", "kind"], &["file", "files"]) {
                    obj.insert("files_only".to_string(), Value::Bool(true));
                } else if string_field_matches(&obj, &["target_kind", "kind"], &["dir", "dirs", "directory", "directories"]) {
                    obj.insert("dirs_only".to_string(), Value::Bool(true));
                }
                obj.entry("names_only".to_string()).or_insert(Value::Bool(true));
                obj.insert(
                    "action".to_string(),
                    Value::String("inventory_dir".to_string()),
                );
                for key in ["by", "mode", "match_kind", "target_kind", "kind", "dir", "directory"] {
                    obj.remove(key);
                }
                return Ok(rewrite_to("system_basic", obj));
            }
            let search_by_ext = obj.get("ext").is_some()
                || string_field_matches(&obj, &["by", "mode", "match_kind"], &["ext", "extension"]);
            if explicit_exact_basename && !search_by_ext {
                obj.entry("exact".to_string()).or_insert(Value::Bool(true));
            }
            obj.insert(
                "action".to_string(),
                Value::String(if search_by_ext { "find_ext" } else { "find_name" }.to_string()),
            );
            for key in ["by", "mode", "match_kind", "path", "dir", "directory"] {
                obj.remove(key);
            }
            Ok(rewrite_to("fs_search", obj))
        }
        "grep_text" => {
            move_value_alias_if_missing(&mut obj, "root", &["path", "dir", "directory"]);
            move_value_alias_if_missing(&mut obj, "query", &["text", "keyword"]);
            move_value_alias_if_missing(&mut obj, "max_results", &["max_matches", "limit"]);
            if obj
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .is_some_and(|case_sensitive| !case_sensitive)
            {
                obj.entry("case_insensitive".to_string())
                    .or_insert(Value::Bool(true));
            }
            promote_grep_pattern_to_query_if_missing(&mut obj);
            drop_redundant_grep_filename_filters(&mut obj);
            obj.insert("action".to_string(), Value::String("grep_text".to_string()));
            for key in ["path", "dir", "directory"] {
                obj.remove(key);
            }
            Ok(rewrite_to("fs_search", obj))
        }
        "compare_paths" => {
            if !obj.contains_key("left_path") || !obj.contains_key("right_path") {
                let path_pair = obj.get("paths").and_then(Value::as_array).and_then(|paths| {
                    (paths.len() == 2).then(|| (paths[0].clone(), paths[1].clone()))
                });
                if let Some((left, right)) = path_pair {
                    obj.entry("left_path".to_string()).or_insert(left);
                    obj.entry("right_path".to_string()).or_insert(right);
                }
            }
            obj.insert(
                "action".to_string(),
                Value::String("compare_paths".to_string()),
            );
            obj.remove("paths");
            Ok(rewrite_to("system_basic", obj))
        }
        "write_text" => {
            move_value_alias_if_missing(&mut obj, "content", &["text", "data", "body"]);
            obj.remove("action");
            Ok(rewrite_to("write_file", obj))
        }
        "append_text" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path", "target"]);
            move_value_alias_if_missing(&mut obj, "content", &["text", "data", "body"]);
            obj.insert("append".to_string(), Value::Bool(true));
            obj.remove("action");
            Ok(rewrite_to("write_file", obj))
        }
        "make_dir" => {
            move_value_alias_if_missing(&mut obj, "path", &["dir", "directory", "target"]);
            obj.remove("action");
            Ok(rewrite_to("make_dir", obj))
        }
        "remove_path" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "target"]);
            obj.remove("action");
            Ok(rewrite_to("remove_file", obj))
        }
        _ => Err(format!(
            "unsupported fs_basic action `{}`; allowed: stat_paths|list_dir|count_entries|read_text_range|find_entries|grep_text|compare_paths|write_text|append_text|make_dir|remove_path",
            action
        )),
    }
}

fn rewrite_config_basic_call(args: Value) -> Result<VirtualToolRewrite, String> {
    let mut obj = args_object(args, "config_basic")?;
    let action = action_name(&obj).unwrap_or_default();
    match action.as_str() {
        "read_field" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            move_value_alias_if_missing(&mut obj, "field_path", &["field", "key"]);
            obj.insert(
                "action".to_string(),
                Value::String("extract_field".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "read_fields" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            move_value_alias_if_missing(&mut obj, "field_paths", &["fields", "keys"]);
            obj.insert(
                "action".to_string(),
                Value::String("extract_fields".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "list_keys" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            obj.insert(
                "action".to_string(),
                Value::String("structured_keys".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "validate" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            obj.insert(
                "action".to_string(),
                Value::String("validate_structured".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "guard_rustclaw_config" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path", "config_path"]);
            obj.entry("path".to_string())
                .or_insert_with(|| Value::String("configs/config.toml".to_string()));
            obj.insert(
                "action".to_string(),
                Value::String("guard_config".to_string()),
            );
            Ok(rewrite_to("config_edit", obj))
        }
        _ => Err(format!(
            "unsupported config_basic action `{}`; allowed: read_field|read_fields|list_keys|validate|guard_rustclaw_config",
            action
        )),
    }
}

fn normalize_fs_basic_args(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    changed |= normalize_action_alias(
        obj,
        &[
            ("stat", "stat_paths"),
            ("metadata", "stat_paths"),
            ("path_facts", "stat_paths"),
            ("list", "list_dir"),
            ("inventory_dir", "list_dir"),
            ("count", "count_entries"),
            ("count_inventory", "count_entries"),
            ("read_range", "read_text_range"),
            ("read_text", "read_text_range"),
            ("find", "find_entries"),
            ("find_name", "find_entries"),
            ("find_path", "find_entries"),
            ("find_ext", "find_entries"),
            ("search", "find_entries"),
            ("grep", "grep_text"),
            ("search_text", "grep_text"),
            ("compare", "compare_paths"),
            ("write_file", "write_text"),
            ("append", "append_text"),
            ("append_file", "append_text"),
            ("append_line", "append_text"),
            ("mkdir", "make_dir"),
            ("remove_file", "remove_path"),
            ("delete_file", "remove_path"),
        ],
    );
    let action = action_name(obj);
    if action.as_deref() == Some("read_text_range") {
        changed |= normalize_read_text_range_args(obj);
    } else if matches!(action.as_deref(), Some("write_text" | "append_text")) {
        changed |= normalize_fs_write_text_args(obj);
    } else {
        changed |= move_value_alias_if_missing(obj, "max_entries", &["limit"]);
    }
    if action.as_deref() == Some("count_entries") {
        changed |= normalize_count_entries_filter_aliases(obj);
    }
    if action.as_deref() == Some("grep_text") {
        changed |= promote_grep_pattern_to_query_if_missing(obj);
        changed |= move_value_alias_if_missing(obj, "query", &["text", "keyword"]);
    }
    changed
}

fn normalize_fs_write_text_args(obj: &mut serde_json::Map<String, Value>) -> bool {
    let mut changed = false;
    changed |= move_value_alias_if_missing(obj, "path", &["file", "file_path", "target"]);
    changed |= move_value_alias_if_missing(obj, "content", &["text", "data", "body"]);
    changed |= move_value_alias_if_missing(obj, "mode", &["write_mode", "writeMode"]);
    changed
}

fn normalize_count_entries_filter_aliases(obj: &mut serde_json::Map<String, Value>) -> bool {
    let mut changed = false;
    let dirs_only = obj
        .get("dirs_only")
        .or_else(|| obj.get("dir_only"))
        .or_else(|| obj.get("directories_only"))
        .or_else(|| obj.get("directory_only"))
        .or_else(|| obj.get("folders_only"))
        .or_else(|| obj.get("folder_only"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let files_only = obj
        .get("files_only")
        .or_else(|| obj.get("file_only"))
        .or_else(|| obj.get("regular_files_only"))
        .or_else(|| obj.get("regular_file_only"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    changed |= move_value_alias_if_missing(
        obj,
        "kind_filter",
        &[
            "filter_kind",
            "target_kind",
            "kind",
            "entry_kind",
            "entry_type",
            "item_kind",
            "item_type",
        ],
    );
    changed |= move_value_alias_if_missing(
        obj,
        "ext_filter",
        &[
            "ext",
            "extension",
            "extensions",
            "file_extension",
            "file_extensions",
        ],
    );
    let kind = obj
        .get("kind_filter")
        .and_then(Value::as_str)
        .map(normalize_tool_token);
    if dirs_only || kind.as_deref().is_some_and(count_kind_is_dir) {
        obj.insert("kind_filter".to_string(), Value::String("dir".to_string()));
        obj.insert("count_dirs".to_string(), Value::Bool(true));
        obj.insert("count_files".to_string(), Value::Bool(false));
        obj.insert("dirs_only".to_string(), Value::Bool(true));
        obj.insert("files_only".to_string(), Value::Bool(false));
        changed = true;
    } else if files_only || kind.as_deref().is_some_and(count_kind_is_file) {
        obj.insert("kind_filter".to_string(), Value::String("file".to_string()));
        obj.insert("count_files".to_string(), Value::Bool(true));
        obj.insert("count_dirs".to_string(), Value::Bool(false));
        obj.insert("files_only".to_string(), Value::Bool(true));
        obj.insert("dirs_only".to_string(), Value::Bool(false));
        changed = true;
    }
    for key in [
        "filter_kind",
        "target_kind",
        "kind",
        "entry_kind",
        "entry_type",
        "item_kind",
        "item_type",
        "dir_only",
        "directories_only",
        "directory_only",
        "folders_only",
        "folder_only",
        "file_only",
        "regular_files_only",
        "regular_file_only",
    ] {
        if obj.remove(key).is_some() {
            changed = true;
        }
    }
    changed
}

fn normalize_tool_token(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn count_kind_is_dir(value: &str) -> bool {
    matches!(
        value,
        "dir"
            | "dirs"
            | "directory"
            | "directories"
            | "folder"
            | "folders"
            | "subdir"
            | "subdirs"
            | "subdirectory"
            | "subdirectories"
            | "subfolder"
            | "subfolders"
    )
}

fn count_kind_is_file(value: &str) -> bool {
    matches!(value, "file" | "files" | "regular_file" | "regular_files")
}

fn normalize_config_basic_args(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = normalize_action_alias(
        obj,
        &[
            ("extract_field", "read_field"),
            ("extract_fields", "read_fields"),
            ("structured_keys", "list_keys"),
            ("keys", "list_keys"),
            ("scan", "guard_rustclaw_config"),
            ("check", "guard_rustclaw_config"),
        ],
    );
    if action_name(obj).as_deref() == Some("guard_rustclaw_config") {
        obj.entry("path".to_string())
            .or_insert_with(|| Value::String("configs/config.toml".to_string()));
        changed = true;
    }
    if action_name(obj).is_none() {
        if has_any_non_empty_arg(obj, &["field_paths", "fields", "keys"]) {
            obj.insert(
                "action".to_string(),
                Value::String("read_fields".to_string()),
            );
            changed = true;
        } else if has_any_non_empty_arg(obj, &["field_path", "field", "key"]) {
            obj.insert(
                "action".to_string(),
                Value::String("read_field".to_string()),
            );
            changed = true;
        }
    }
    changed
}

fn args_object(args: Value, tool: &str) -> Result<serde_json::Map<String, Value>, String> {
    args.as_object()
        .cloned()
        .ok_or_else(|| format!("{tool} args must be a JSON object"))
}

fn action_name(obj: &serde_json::Map<String, Value>) -> Option<String> {
    obj.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn rewrite_to(tool: &str, obj: serde_json::Map<String, Value>) -> VirtualToolRewrite {
    VirtualToolRewrite {
        runtime_tool: tool.to_string(),
        runtime_args: Value::Object(obj),
    }
}

fn move_value_alias_if_missing(
    obj: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) -> bool {
    if obj.get(canonical).is_some() {
        return false;
    }
    let Some((alias, value)) = aliases
        .iter()
        .find_map(|alias| obj.get(*alias).cloned().map(|value| (*alias, value)))
    else {
        return false;
    };
    obj.insert(canonical.to_string(), value);
    obj.remove(alias);
    true
}

fn normalize_read_text_range_args(obj: &mut serde_json::Map<String, Value>) -> bool {
    let mut changed = false;
    changed |= move_value_alias_if_missing(
        obj,
        "start_line",
        &["line_start", "from_line", "start", "from", "offset"],
    );
    changed |= move_value_alias_if_missing(obj, "end_line", &["line_end", "to_line", "end", "to"]);
    changed |= move_value_alias_if_missing(
        obj,
        "field_selector",
        &["selector", "field", "structured_field_selector"],
    );
    if let Some(normalized) = obj
        .get("field_selector")
        .and_then(Value::as_str)
        .map(normalize_tool_token)
        .filter(|value| !value.is_empty())
    {
        if obj.get("field_selector").and_then(Value::as_str) != Some(normalized.as_str()) {
            obj.insert("field_selector".to_string(), Value::String(normalized));
            changed = true;
        }
    }

    let count = read_nonzero_u64_arg(obj, &["n", "line_count", "lines", "count", "limit"]);
    if obj.get("end_line").is_none() {
        if let (Some(start), Some(count)) = (read_u64_arg(obj, "start_line"), count) {
            let start = start.max(1);
            obj.insert("start_line".to_string(), Value::from(start));
            obj.insert(
                "end_line".to_string(),
                Value::from(start.saturating_add(count).saturating_sub(1)),
            );
            changed = true;
        } else if obj.get("n").is_none() {
            if let Some(count) = count {
                obj.insert("n".to_string(), Value::from(count));
                changed = true;
            }
        }
    }

    if obj.get("mode").is_none()
        && (obj.get("start_line").is_some() || obj.get("end_line").is_some())
    {
        obj.insert("mode".to_string(), Value::String("range".to_string()));
        changed = true;
    }
    for key in [
        "offset",
        "limit",
        "line_count",
        "lines",
        "count",
        "start",
        "from",
        "end",
        "to",
    ] {
        if obj.remove(key).is_some() {
            changed = true;
        }
    }
    changed
}

fn read_nonzero_u64_arg(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| read_u64_arg(obj, key))
        .filter(|value| *value > 0)
}

fn read_u64_arg(obj: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    match obj.get(key) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(value)) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn move_existing_directory_alias_to_root(
    obj: &mut serde_json::Map<String, Value>,
    aliases: &[&str],
) -> bool {
    if obj.get("root").is_some() {
        return false;
    }
    let Some((alias, value)) = aliases.iter().find_map(|alias| {
        obj.get(*alias)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && looks_like_directory_root_reference(value))
            .and_then(|_| obj.get(*alias).cloned().map(|value| (*alias, value)))
    }) else {
        return false;
    };
    obj.insert("root".to_string(), value);
    obj.remove(alias);
    true
}

fn looks_like_directory_root_reference(value: &str) -> bool {
    let path = Path::new(value);
    if path.is_dir() {
        return true;
    }
    let has_path_separator = value.contains('/') || value.contains('\\');
    if !has_path_separator || value.contains(['*', '?']) {
        return false;
    }
    path.extension().is_none()
}

fn normalize_action_alias(
    obj: &mut serde_json::Map<String, Value>,
    aliases: &[(&str, &str)],
) -> bool {
    let Some(action) = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let action_lc = action.to_ascii_lowercase();
    let Some((_, target)) = aliases.iter().find(|(alias, _)| action_lc == *alias) else {
        return false;
    };
    if action == *target {
        return false;
    }
    obj.insert("action".to_string(), Value::String((*target).to_string()));
    true
}

fn string_field_matches(
    obj: &serde_json::Map<String, Value>,
    fields: &[&str],
    allowed_values: &[&str],
) -> bool {
    fields.iter().any(|field| {
        obj.get(*field)
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| {
                allowed_values
                    .iter()
                    .any(|allowed| value.eq_ignore_ascii_case(allowed))
            })
    })
}

fn has_non_empty_arg(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    match obj.get(key) {
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

fn has_any_non_empty_arg(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|key| has_non_empty_arg(obj, key))
}

fn explicit_find_entries_exact_basename_selector(obj: &serde_json::Map<String, Value>) -> bool {
    [
        "name_pattern",
        "basename_pattern",
        "filename_pattern",
        "filename",
        "entry_name",
    ]
    .iter()
    .any(|key| obj.get(*key).is_some_and(concrete_basename_selector_value))
}

fn concrete_basename_selector_value(value: &Value) -> bool {
    match value {
        Value::String(text) => concrete_basename_selector_text(text),
        Value::Array(items) if !items.is_empty() => items
            .iter()
            .all(|item| item.as_str().is_some_and(concrete_basename_selector_text)),
        _ => false,
    }
}

fn concrete_basename_selector_text(text: &str) -> bool {
    let trimmed = text.trim().trim_matches('"').trim_matches('\'').trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(['*', '?', '[', ']', '(', ')', '{', '}', '|'])
    {
        return false;
    }
    Path::new(trimmed)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .is_some_and(|ext| !ext.is_empty())
}

fn promote_globish_pattern_to_ext(obj: &mut serde_json::Map<String, Value>) -> bool {
    if has_non_empty_arg(obj, "ext") {
        return false;
    }
    let Some(ext) = obj
        .get("pattern")
        .and_then(Value::as_str)
        .and_then(extension_from_globish_filter)
    else {
        return false;
    };
    obj.insert("ext".to_string(), Value::String(ext));
    obj.remove("pattern");
    true
}

fn promote_pure_extension_pattern_to_ext(obj: &mut serde_json::Map<String, Value>) -> bool {
    if has_non_empty_arg(obj, "ext") {
        return false;
    }
    let Some(ext) = obj
        .get("pattern")
        .and_then(Value::as_str)
        .and_then(extension_from_pure_extension_filter)
    else {
        return false;
    };
    obj.insert("ext".to_string(), Value::String(ext));
    obj.remove("pattern");
    true
}

fn extension_from_pure_extension_filter(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase();
    let ext = cleaned
        .strip_prefix("*.")
        .or_else(|| cleaned.strip_prefix('.'))?;
    if ext.is_empty()
        || ext.contains(['*', '?', '.', '/', '\\'])
        || !ext
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    Some(ext.to_string())
}

fn extension_from_globish_filter(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase();
    let (_prefix, ext) = cleaned.rsplit_once('.')?;
    if ext.is_empty()
        || ext.contains(['*', '?', '/', '\\'])
        || !ext
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    cleaned
        .contains('*')
        .then(|| ext.to_string())
        .or_else(|| cleaned.strip_prefix('.').map(ToString::to_string))
}

fn extension_selector_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => extension_from_globish_filter(text).map(Value::String),
        Value::Array(items) => {
            if items.is_empty() {
                return None;
            }
            let mut extensions = Vec::with_capacity(items.len());
            for item in items {
                let ext = item.as_str().and_then(extension_from_globish_filter)?;
                extensions.push(Value::String(ext));
            }
            Some(Value::Array(extensions))
        }
        _ => None,
    }
}

fn promote_extension_names_to_ext(obj: &mut serde_json::Map<String, Value>) -> bool {
    if has_non_empty_arg(obj, "ext") || has_non_empty_arg(obj, "pattern") {
        return false;
    }
    let Some(ext) = obj.get("names").and_then(extension_selector_value) else {
        return false;
    };
    obj.insert("ext".to_string(), ext);
    obj.remove("names");
    true
}

fn demote_existing_directory_pattern_to_root(obj: &mut serde_json::Map<String, Value>) -> bool {
    let root_is_default = obj
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|value| value.is_empty() || value == ".")
        .unwrap_or(true);
    if !root_is_default {
        return false;
    }
    let Some(pattern) = obj
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let path = Path::new(pattern);
    if !path.is_dir() || path.components().count() <= 1 {
        return false;
    }
    obj.insert("root".to_string(), Value::String(pattern.to_string()));
    obj.remove("pattern");
    true
}

fn drop_redundant_grep_filename_filters(obj: &mut serde_json::Map<String, Value>) -> bool {
    let Some(query) = obj
        .get("query")
        .and_then(Value::as_str)
        .map(normalized_filter_token)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let mut changed = false;
    if obj
        .get("pattern")
        .and_then(Value::as_str)
        .map(normalized_filter_token)
        .is_some_and(|pattern| pattern == query)
    {
        obj.remove("pattern");
        changed = true;
    }
    match obj.remove("patterns") {
        Some(Value::Array(values)) => {
            let original_len = values.len();
            let retained = values
                .into_iter()
                .filter(|value| {
                    value
                        .as_str()
                        .map(normalized_filter_token)
                        .map_or(true, |pattern| pattern != query)
                })
                .collect::<Vec<_>>();
            if retained.is_empty() {
                changed = true;
            } else {
                changed |= retained.len() != original_len;
                obj.insert("patterns".to_string(), Value::Array(retained));
            }
        }
        Some(Value::String(value)) => {
            if normalized_filter_token(&value) == query {
                changed = true;
            } else {
                obj.insert("patterns".to_string(), Value::String(value));
            }
        }
        Some(value) => {
            obj.insert("patterns".to_string(), value);
        }
        None => {}
    }
    changed
}

fn normalized_filter_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn promote_grep_pattern_to_query_if_missing(obj: &mut serde_json::Map<String, Value>) -> bool {
    if has_non_empty_arg(obj, "query") {
        return false;
    }
    let Some(pattern) = obj.remove("pattern") else {
        return false;
    };
    if matches!(&pattern, Value::String(value) if value.trim().is_empty()) {
        return false;
    }
    obj.insert("query".to_string(), pattern);
    true
}

#[cfg(test)]
#[path = "virtual_tools_tests.rs"]
mod tests;
