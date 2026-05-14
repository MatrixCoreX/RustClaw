use serde_json::Value;

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

pub(crate) fn is_planner_facing_virtual_tool(tool: &str) -> bool {
    matches!(tool, "fs_basic" | "config_basic")
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
        Value::String("guard_rustclaw_config".to_string()),
    );
    Some(VirtualToolCanonicalCall {
        tool: "config_basic".to_string(),
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
            obj.insert(
                "action".to_string(),
                Value::String("count_inventory".to_string()),
            );
            Ok(rewrite_to("system_basic", obj))
        }
        "read_text_range" => {
            move_value_alias_if_missing(&mut obj, "path", &["file", "file_path"]);
            obj.insert("action".to_string(), Value::String("read_range".to_string()));
            Ok(rewrite_to("system_basic", obj))
        }
        "find_entries" => {
            move_value_alias_if_missing(&mut obj, "root", &["path", "dir", "directory"]);
            move_value_alias_if_missing(
                &mut obj,
                "pattern",
                &["name", "keyword", "query", "filename"],
            );
            move_value_alias_if_missing(&mut obj, "ext", &["extension", "ext_filter"]);
            let has_pattern = has_non_empty_arg(&obj, "pattern");
            let has_ext = has_non_empty_arg(&obj, "ext");
            if !has_pattern && !has_ext {
                if let Some(root) = obj.remove("root") {
                    obj.insert("path".to_string(), root);
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
            "unsupported fs_basic action `{}`; allowed: stat_paths|list_dir|count_entries|read_text_range|find_entries|grep_text|compare_paths|write_text|make_dir|remove_path",
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
            obj.remove("action");
            Ok(rewrite_to("config_guard", obj))
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
            ("find_ext", "find_entries"),
            ("search", "find_entries"),
            ("grep", "grep_text"),
            ("search_text", "grep_text"),
            ("compare", "compare_paths"),
            ("write_file", "write_text"),
            ("mkdir", "make_dir"),
            ("remove_file", "remove_path"),
            ("delete_file", "remove_path"),
        ],
    );
    changed |= move_value_alias_if_missing(obj, "max_entries", &["limit"]);
    changed
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
mod tests {
    use super::{
        canonicalize_legacy_tool_call, normalize_virtual_tool_arg_aliases,
        rewrite_virtual_tool_call,
    };
    use serde_json::json;

    #[test]
    fn legacy_system_basic_path_batch_facts_canonicalizes_to_fs_basic() {
        let canonical = canonicalize_legacy_tool_call(
            "system_basic",
            json!({"action":"path_batch_facts", "paths":["README.md"]}),
        )
        .expect("canonical");
        assert_eq!(canonical.tool, "fs_basic");
        assert_eq!(
            canonical.args.get("action").and_then(|v| v.as_str()),
            Some("stat_paths")
        );
    }

    #[test]
    fn legacy_system_basic_count_inventory_canonicalizes_to_fs_basic_count_entries() {
        let canonical = canonicalize_legacy_tool_call(
            "system_basic",
            json!({"action":"count_inventory", "path":"scripts"}),
        )
        .expect("canonical");
        assert_eq!(canonical.tool, "fs_basic");
        assert_eq!(
            canonical.args.get("action").and_then(|v| v.as_str()),
            Some("count_entries")
        );
    }

    #[test]
    fn legacy_system_basic_extract_field_canonicalizes_to_config_basic() {
        let canonical = canonicalize_legacy_tool_call(
            "system_basic",
            json!({"action":"extract_field", "path":"Cargo.toml", "field_path":"workspace.package.version"}),
        )
        .expect("canonical");
        assert_eq!(canonical.tool, "config_basic");
        assert_eq!(
            canonical.args.get("action").and_then(|v| v.as_str()),
            Some("read_field")
        );
    }

    #[test]
    fn legacy_fs_search_find_ext_canonicalizes_to_fs_basic_find_entries() {
        let canonical = canonicalize_legacy_tool_call(
            "fs_search",
            json!({"action":"find_ext", "root":"scripts", "ext":"sh"}),
        )
        .expect("canonical");
        assert_eq!(canonical.tool, "fs_basic");
        assert_eq!(
            canonical.args.get("action").and_then(|v| v.as_str()),
            Some("find_entries")
        );
        assert_eq!(
            canonical.args.get("ext").and_then(|v| v.as_str()),
            Some("sh")
        );
    }

    #[test]
    fn legacy_fs_search_grep_text_drops_query_as_filename_filter() {
        let canonical = canonicalize_legacy_tool_call(
            "fs_search",
            json!({
                "action": "grep_text",
                "root": ".",
                "query": "FirstLayerDecision",
                "pattern": "FirstLayerDecision",
                "patterns": ["FirstLayerDecision", "*.rs"]
            }),
        )
        .expect("canonical");

        assert_eq!(canonical.tool, "fs_basic");
        assert_eq!(
            canonical.args.get("action").and_then(|v| v.as_str()),
            Some("grep_text")
        );
        assert!(canonical.args.get("pattern").is_none());
        assert_eq!(canonical.args.get("patterns"), Some(&json!(["*.rs"])));
    }

    #[test]
    fn fs_basic_stat_paths_rewrites_to_system_basic_path_batch_facts() {
        let mut args = json!({"action":"stat", "path":"README.md"});
        assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "system_basic");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("path_batch_facts")
        );
        assert_eq!(
            rewrite.runtime_args.get("paths").and_then(|v| v.as_str()),
            Some("README.md")
        );
    }

    #[test]
    fn fs_basic_count_entries_rewrites_to_system_basic_count_inventory() {
        let mut args = json!({"action":"count", "directory":"scripts"});
        assert!(normalize_virtual_tool_arg_aliases("fs_basic", &mut args));
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "system_basic");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("count_inventory")
        );
        assert_eq!(
            rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
            Some("scripts")
        );
    }

    #[test]
    fn fs_basic_find_entries_by_extension_rewrites_to_fs_search_find_ext() {
        let args = json!({"action":"find_entries", "root":"scripts", "extension":"sh"});
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "fs_search");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("find_ext")
        );
        assert_eq!(
            rewrite.runtime_args.get("ext").and_then(|v| v.as_str()),
            Some("sh")
        );
    }

    #[test]
    fn fs_basic_find_entries_ext_filter_alias_rewrites_to_find_ext() {
        let args = json!({"action":"find_entries", "root":"scripts", "ext_filter":"sh"});
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "fs_search");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("find_ext")
        );
        assert_eq!(
            rewrite.runtime_args.get("ext").and_then(|v| v.as_str()),
            Some("sh")
        );
    }

    #[test]
    fn fs_basic_grep_text_drops_redundant_query_pattern_before_runtime() {
        let args = json!({
            "action": "grep_text",
            "root": ".",
            "query": "FirstLayerDecision",
            "pattern": "FirstLayerDecision"
        });
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");

        assert_eq!(rewrite.runtime_tool, "fs_search");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("grep_text")
        );
        assert_eq!(
            rewrite.runtime_args.get("query").and_then(|v| v.as_str()),
            Some("FirstLayerDecision")
        );
        assert!(rewrite.runtime_args.get("pattern").is_none());
    }

    #[test]
    fn fs_basic_grep_text_promotes_single_pattern_to_content_query() {
        let args = json!({
            "action": "grep_text",
            "path": ".",
            "pattern": "FirstLayerDecision"
        });
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");

        assert_eq!(rewrite.runtime_tool, "fs_search");
        assert_eq!(
            rewrite.runtime_args.get("query").and_then(|v| v.as_str()),
            Some("FirstLayerDecision")
        );
        assert!(rewrite.runtime_args.get("pattern").is_none());
        assert_eq!(
            rewrite.runtime_args.get("root").and_then(|v| v.as_str()),
            Some(".")
        );
    }

    #[test]
    fn fs_basic_find_entries_without_criterion_degrades_to_directory_listing() {
        let args = json!({"action":"find_entries", "path":"plan", "target_kind":"file"});
        let rewrite = rewrite_virtual_tool_call("fs_basic", args)
            .unwrap()
            .expect("rewrite");

        assert_eq!(rewrite.runtime_tool, "system_basic");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("inventory_dir")
        );
        assert_eq!(
            rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
            Some("plan")
        );
        assert_eq!(
            rewrite
                .runtime_args
                .get("files_only")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn config_basic_read_field_rewrites_to_system_basic_extract_field() {
        let mut args = json!({"action":"extract_field", "file":"Cargo.toml", "key":"workspace.package.version"});
        assert!(normalize_virtual_tool_arg_aliases(
            "config_basic",
            &mut args
        ));
        let rewrite = rewrite_virtual_tool_call("config_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "system_basic");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("extract_field")
        );
        assert_eq!(
            rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
            Some("Cargo.toml")
        );
        assert_eq!(
            rewrite
                .runtime_args
                .get("field_path")
                .and_then(|v| v.as_str()),
            Some("workspace.package.version")
        );
    }

    #[test]
    fn config_basic_missing_action_with_field_path_defaults_to_read_field() {
        let mut args = json!({"path":"/tmp/package.json", "field_path":"name", "format":"json"});
        assert!(normalize_virtual_tool_arg_aliases(
            "config_basic",
            &mut args
        ));
        let rewrite = rewrite_virtual_tool_call("config_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "system_basic");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("extract_field")
        );
        assert_eq!(
            rewrite
                .runtime_args
                .get("field_path")
                .and_then(|v| v.as_str()),
            Some("name")
        );
    }

    #[test]
    fn config_basic_guard_rewrites_to_config_guard() {
        let args = json!({"action":"guard_rustclaw_config", "path":"configs/config.toml"});
        let rewrite = rewrite_virtual_tool_call("config_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "config_guard");
        assert!(rewrite.runtime_args.get("action").is_none());
    }

    #[test]
    fn config_basic_validate_rewrites_to_structured_validation() {
        let args = json!({"action":"validate", "path":"configs/config.toml", "format":"toml"});
        let rewrite = rewrite_virtual_tool_call("config_basic", args)
            .unwrap()
            .expect("rewrite");
        assert_eq!(rewrite.runtime_tool, "system_basic");
        assert_eq!(
            rewrite.runtime_args.get("action").and_then(|v| v.as_str()),
            Some("validate_structured")
        );
        assert_eq!(
            rewrite.runtime_args.get("path").and_then(|v| v.as_str()),
            Some("configs/config.toml")
        );
    }
}
