use super::*;

pub(super) fn string_list_from_value(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(item)) => {
            let item = item.trim();
            if item.is_empty() {
                Vec::new()
            } else {
                vec![item.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

pub(super) fn parse_positive_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().map(|n| n as usize).filter(|n| *n > 0),
        Value::String(text) => text.trim().parse::<usize>().ok().filter(|n| *n > 0),
        _ => None,
    }
}

pub(super) fn parse_i64_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub(super) fn parse_line_range_text(text: &str) -> Option<(usize, usize)> {
    let nums = text
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .collect::<Vec<_>>();
    match nums.as_slice() {
        [end] => Some((1, *end)),
        [start, end, ..] => Some((*start, (*end).max(*start))),
        _ => None,
    }
}

pub(super) fn parse_line_range_value(value: &Value) -> Option<(usize, usize)> {
    match value {
        Value::String(text) => parse_line_range_text(text),
        Value::Array(items) => {
            if items.is_empty() {
                return None;
            }
            let start = parse_positive_usize(items.first()?)?;
            let end = items.get(1).and_then(parse_positive_usize).unwrap_or(start);
            Some((start, end.max(start)))
        }
        Value::Object(obj) => {
            let start = obj
                .get("start_line")
                .or_else(|| obj.get("start"))
                .or_else(|| obj.get("from"))
                .and_then(parse_positive_usize)
                .unwrap_or(1);
            let end = obj
                .get("end_line")
                .or_else(|| obj.get("end"))
                .or_else(|| obj.get("to"))
                .and_then(parse_positive_usize)?;
            Some((start, end.max(start)))
        }
        Value::Number(_) => parse_positive_usize(value).map(|end| (1, end)),
        _ => None,
    }
}

pub(super) fn normalize_read_range_negative_bounds(
    obj: &mut serde_json::Map<String, Value>,
) -> bool {
    let Some(start) = obj.get("start_line").and_then(parse_i64_value) else {
        return false;
    };
    let Some(end) = obj.get("end_line").and_then(parse_i64_value) else {
        return false;
    };
    if start >= 0 || end >= 0 || start > end {
        return false;
    }
    let n = end.saturating_sub(start).saturating_add(1);
    if n <= 0 {
        return false;
    }
    obj.insert("mode".to_string(), Value::String("tail".to_string()));
    obj.insert(
        "n".to_string(),
        Value::Number(serde_json::Number::from(n as u64)),
    );
    obj.remove("start_line");
    obj.remove("end_line");
    true
}

pub(super) fn line_count_template_tail_n(start: &str, end: &str) -> Option<usize> {
    let start = start.trim();
    let end = end.trim();
    if !start.contains("line_count") || !end.contains("line_count") {
        return None;
    }
    let marker = start.rsplit_once('-')?.1.trim();
    let offset = marker
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<usize>()
        .ok()?;
    Some(offset.saturating_add(1)).filter(|n| *n > 0)
}

pub(super) fn normalize_read_range_line_count_template(
    obj: &mut serde_json::Map<String, Value>,
) -> bool {
    let Some(start) = obj.get("start_line").and_then(Value::as_str) else {
        return false;
    };
    let Some(end) = obj.get("end_line").and_then(Value::as_str) else {
        return false;
    };
    let Some(n) = line_count_template_tail_n(start, end) else {
        return false;
    };
    obj.insert("mode".to_string(), Value::String("tail".to_string()));
    obj.insert(
        "n".to_string(),
        Value::Number(serde_json::Number::from(n as u64)),
    );
    obj.remove("start_line");
    obj.remove("end_line");
    true
}

pub(super) fn normalize_read_range_negative_start_count(
    obj: &mut serde_json::Map<String, Value>,
) -> bool {
    let Some(start) = obj.get("start_line").and_then(parse_i64_value) else {
        return false;
    };
    if start >= 0 {
        return false;
    }
    let n = obj
        .get("line_count")
        .or_else(|| obj.get("count"))
        .or_else(|| obj.get("limit"))
        .or_else(|| obj.get("n"))
        .and_then(parse_i64_value)
        .unwrap_or_else(|| start.saturating_abs());
    if n <= 0 {
        return false;
    }
    obj.insert("mode".to_string(), Value::String("tail".to_string()));
    obj.insert(
        "n".to_string(),
        Value::Number(serde_json::Number::from(n as u64)),
    );
    obj.remove("start_line");
    obj.remove("end_line");
    obj.remove("line_count");
    obj.remove("count");
    obj.remove("limit");
    true
}

pub(super) fn has_non_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(has_non_empty_json_value),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

pub(super) fn normalize_inventory_dir_sort_by_value(
    obj: &serde_json::Map<String, Value>,
) -> Option<String> {
    let sort_by = obj
        .get("sort_by")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    let order_descending = obj
        .get("order")
        .or_else(|| obj.get("sort_order"))
        .or_else(|| obj.get("direction"))
        .and_then(Value::as_str)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "asc" | "ascending")
        });
    match sort_by.as_str() {
        "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc" | "name_desc" => Some(sort_by),
        "name" => Some(if order_descending == Some(true) {
            "name_desc".to_string()
        } else {
            "name".to_string()
        }),
        "mtime" | "modified" | "modified_ts" | "modified_time" => {
            Some(if order_descending.unwrap_or(true) {
                "mtime_desc".to_string()
            } else {
                "mtime_asc".to_string()
            })
        }
        "size" | "size_bytes" | "bytes" => Some(if order_descending.unwrap_or(true) {
            "size_desc".to_string()
        } else {
            "size_asc".to_string()
        }),
        _ => None,
    }
}

pub(super) fn normalize_read_range_line_aliases(obj: &mut serde_json::Map<String, Value>) {
    normalize_arg_alias(obj, "start_line", &["line_start", "from_line"]);
    normalize_arg_alias(obj, "end_line", &["line_end", "to_line"]);
    if obj.get("start_line").is_some_and(has_non_empty_json_value)
        && obj.get("end_line").is_some_and(has_non_empty_json_value)
    {
        obj.entry("mode".to_string())
            .or_insert_with(|| Value::String("range".to_string()));
    }
    let Some(range_value) = obj
        .remove("lines")
        .or_else(|| obj.remove("line_range"))
        .or_else(|| obj.remove("range"))
    else {
        return;
    };
    if let Some(mode) = range_value
        .as_str()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|mode| matches!(mode.as_str(), "head" | "tail" | "full" | "all"))
    {
        obj.insert("mode".to_string(), Value::String(mode));
        return;
    }
    let Some((start, end)) = parse_line_range_value(&range_value) else {
        return;
    };
    obj.insert("mode".to_string(), Value::String("range".to_string()));
    obj.insert(
        "start_line".to_string(),
        Value::Number(serde_json::Number::from(start as u64)),
    );
    obj.insert(
        "end_line".to_string(),
        Value::Number(serde_json::Number::from(end as u64)),
    );
    obj.entry("n".to_string()).or_insert_with(|| {
        Value::Number(serde_json::Number::from(
            end.saturating_sub(start).saturating_add(1) as u64,
        ))
    });
}

pub(super) fn normalize_path_alias_to_path(
    obj: &mut serde_json::Map<String, Value>,
    aliases: &[&str],
) {
    if obj.get("path").is_some_and(has_non_empty_json_value) {
        return;
    }
    for alias in aliases {
        let Some(value) = obj.remove(*alias) else {
            continue;
        };
        if has_non_empty_json_value(&value) {
            obj.insert("path".to_string(), value);
            return;
        }
    }
}

pub(super) fn normalize_arg_alias(
    obj: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) {
    if obj.get(canonical).is_some_and(has_non_empty_json_value) {
        return;
    }
    for alias in aliases {
        let Some(value) = obj.remove(*alias) else {
            continue;
        };
        if has_non_empty_json_value(&value) {
            obj.insert(canonical.to_string(), value);
            return;
        }
    }
}

pub(super) fn normalize_path_batch_facts_args(obj: &mut serde_json::Map<String, Value>) {
    if obj.contains_key("paths") {
        return;
    }
    if let Some(paths) = obj
        .remove("targets")
        .or_else(|| obj.remove("target_paths"))
        .or_else(|| obj.remove("path_list"))
        .or_else(|| obj.remove("path_array"))
    {
        obj.insert("paths".to_string(), paths);
    } else if let Some(path) = obj.remove("path") {
        obj.insert("paths".to_string(), Value::Array(vec![path]));
    }
}

fn normalize_compare_paths_args(obj: &mut serde_json::Map<String, Value>) {
    if !obj.contains_key("left_path") {
        if let Some(value) = obj
            .remove("path1")
            .or_else(|| obj.remove("path_a"))
            .or_else(|| obj.remove("left"))
            .or_else(|| obj.remove("source_path"))
            .or_else(|| obj.remove("first_path"))
        {
            obj.insert("left_path".to_string(), value);
        }
    }
    if !obj.contains_key("right_path") {
        if let Some(value) = obj
            .remove("path2")
            .or_else(|| obj.remove("path_b"))
            .or_else(|| obj.remove("right"))
            .or_else(|| obj.remove("target_path"))
            .or_else(|| obj.remove("second_path"))
        {
            obj.insert("right_path".to_string(), value);
        }
    }
    if obj.contains_key("left_path") && obj.contains_key("right_path") {
        return;
    }
    let paths = string_list_from_value(obj.get("paths"))
        .into_iter()
        .chain(string_list_from_value(obj.get("targets")))
        .collect::<Vec<_>>();
    if paths.len() >= 2 {
        obj.entry("left_path".to_string())
            .or_insert_with(|| Value::String(paths[0].clone()));
        obj.entry("right_path".to_string())
            .or_insert_with(|| Value::String(paths[1].clone()));
    }
}

pub(super) fn normalize_system_basic_args(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match action_name.as_str() {
        "read" | "read_file" => {
            obj.insert(
                "action".to_string(),
                Value::String("read_range".to_string()),
            );
            normalize_read_range_line_aliases(obj);
            normalize_read_range_negative_start_count(obj);
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "read_range" => {
            normalize_read_range_line_aliases(obj);
            normalize_read_range_negative_start_count(obj);
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "list" | "list_dir" | "ls" => {
            obj.insert(
                "action".to_string(),
                Value::String("inventory_dir".to_string()),
            );
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
            obj.entry("names_only".to_string())
                .or_insert_with(|| Value::Bool(true));
        }
        "count_dir" | "count_directory" | "count_children" | "count_entries" | "count_items"
        | "directory_count" | "dir_count" | "inventory_count" => {
            obj.insert(
                "action".to_string(),
                Value::String("count_inventory".to_string()),
            );
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
        }
        "count_inventory" => {
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
        }
        "check_exists" | "exists" | "path_exists" | "stat_paths" => {
            obj.insert(
                "action".to_string(),
                Value::String("path_batch_facts".to_string()),
            );
            normalize_path_alias_to_path(
                obj,
                &[
                    "target",
                    "target_path",
                    "file",
                    "file_path",
                    "dir_path",
                    "directory_path",
                    "directory",
                    "dir",
                ],
            );
            normalize_path_batch_facts_args(obj);
        }
        "path_batch_facts" => {
            normalize_path_batch_facts_args(obj);
        }
        "find_name" => {
            obj.insert("action".to_string(), Value::String("find_path".to_string()));
            normalize_arg_alias(obj, "name", &["pattern", "query", "target", "keyword"]);
        }
        "find_path" => {
            normalize_arg_alias(obj, "name", &["query", "target", "keyword", "name_pattern"]);
        }
        "inventory_dir" => {
            normalize_path_alias_to_path(obj, &["dir_path", "directory_path", "directory", "dir"]);
            let has_extension_filter = obj.get("ext_filter").is_some_and(has_non_empty_json_value)
                || obj.get("extension").is_some_and(has_non_empty_json_value)
                || obj.get("extensions").is_some_and(has_non_empty_json_value);
            let dirs_only = obj
                .get("dirs_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if has_extension_filter && !dirs_only {
                obj.insert("files_only".to_string(), Value::Bool(true));
            }
            if !obj.contains_key("ext_filter") {
                if let Some(value) = obj.remove("extension").or_else(|| obj.remove("extensions")) {
                    obj.insert("ext_filter".to_string(), value);
                }
            }
            if let Some(sort_by) = normalize_inventory_dir_sort_by_value(obj) {
                obj.insert("sort_by".to_string(), Value::String(sort_by));
            }
        }
        "compare_paths" => {
            normalize_compare_paths_args(obj);
        }
        _ => {}
    }
    args
}

pub(super) fn normalize_fs_basic_args_for_planner(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match action_name.as_str() {
        "read_text_range" => {
            normalize_read_range_line_aliases(obj);
            normalize_read_range_negative_start_count(obj);
            normalize_read_range_negative_bounds(obj);
            normalize_read_range_line_count_template(obj);
        }
        "append_text" => {
            normalize_path_alias_to_path(obj, &["file", "file_path", "target"]);
            normalize_arg_alias(obj, "content", &["text", "data", "body", "line"]);
        }
        "write_text" => {
            normalize_path_alias_to_path(obj, &["file", "file_path", "target"]);
            normalize_arg_alias(obj, "content", &["text", "data", "body"]);
        }
        "grep_text" => {
            if obj
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .is_some_and(|case_sensitive| !case_sensitive)
            {
                obj.entry("case_insensitive".to_string())
                    .or_insert(Value::Bool(true));
            }
            normalize_arg_alias(obj, "max_results", &["max_matches", "limit"]);
        }
        "compare_paths" => {
            normalize_compare_paths_args(obj);
        }
        _ => {}
    }
    args
}

pub(super) fn normalize_fs_basic_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("fs_basic") => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_fs_basic_args_for_planner(args),
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("fs_basic") => {
                AgentAction::CallTool {
                    tool,
                    args: normalize_fs_basic_args_for_planner(args),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_system_basic_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "system_basic" => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_system_basic_args(args),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_git_basic_args(
    mut args: Value,
    route_result: Option<&RouteResult>,
) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('-', "_");

    if normalize_git_show_file_at_rev_args(obj, &action_name) {
        info!(
            "plan_normalize_git_basic_action_alias action={} normalized=show_file_at_rev",
            action_name
        );
        return args;
    }

    let normalized = match action_name.as_str() {
        "branches" | "list_branches" | "all_branches" => {
            if route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::Scalar
                )
            }) {
                "current_branch"
            } else {
                "branch"
            }
        }
        "current_branch_name" | "branch_current" | "get_current_branch" => "current_branch",
        "cached_diff" | "staged_diff" => "diff_cached",
        "changed_file" | "changed_file_names" => "changed_files",
        "revparse" | "head" => "rev_parse",
        _ => return args,
    };
    obj.insert("action".to_string(), Value::String(normalized.to_string()));
    info!(
        "plan_normalize_git_basic_action_alias action={} normalized={}",
        action_name, normalized
    );
    args
}

fn normalize_git_show_file_at_rev_args(
    obj: &mut serde_json::Map<String, Value>,
    action_name: &str,
) -> bool {
    if !matches!(
        action_name,
        "show" | "show_file" | "show_file_at_rev" | "show_file_at_revision"
    ) {
        return false;
    }
    if !obj.contains_key("target") {
        if let Some(revision) = obj
            .get("ref")
            .or_else(|| obj.get("revision"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            obj.insert("target".to_string(), Value::String(revision.to_string()));
        }
    }
    if obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        obj.insert(
            "action".to_string(),
            Value::String("show_file_at_rev".to_string()),
        );
        return true;
    }
    let Some(target) = obj
        .get("target")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    else {
        return false;
    };
    let Some((revision, path)) = target.split_once(':') else {
        return false;
    };
    let revision = revision.trim().to_string();
    let path = path.trim().to_string();
    if revision.is_empty() || path.is_empty() {
        return false;
    }
    obj.insert(
        "action".to_string(),
        Value::String("show_file_at_rev".to_string()),
    );
    obj.insert("target".to_string(), Value::String(revision));
    obj.insert("path".to_string(), Value::String(path));
    true
}

pub(super) fn normalize_git_basic_schema_aliases(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("git_basic") => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_git_basic_args(args, route_result),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn rewrite_readonly_git_run_cmd_to_git_basic(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !git_basic_available_for_plan(state) {
        return actions;
    }
    if matches!(
        route_result.map(|route| route.effective_output_contract_semantic_kind()),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if state.resolve_canonical_skill_name(&skill) == "run_cmd" =>
            {
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || !run_cmd_git_command_scope_matches_workspace(state, &args)
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                let Some(git_args) = git_basic_args_from_readonly_git_command(command) else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                AgentAction::CallSkill {
                    skill: "git_basic".to_string(),
                    args: git_args,
                }
            }
            AgentAction::CallTool { tool, args }
                if state.resolve_canonical_skill_name(&tool) == "run_cmd" =>
            {
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || !run_cmd_git_command_scope_matches_workspace(state, &args)
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                let Some(git_args) = git_basic_args_from_readonly_git_command(command) else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                AgentAction::CallSkill {
                    skill: "git_basic".to_string(),
                    args: git_args,
                }
            }
            AgentAction::CallCapability { capability, args }
                if matches!(capability.as_str(), "system.run_command" | "run_cmd") =>
            {
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || !run_cmd_git_command_scope_matches_workspace(state, &args)
                {
                    return AgentAction::CallCapability { capability, args };
                }
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallCapability { capability, args };
                };
                let Some(git_args) = git_basic_args_from_readonly_git_command(command) else {
                    return AgentAction::CallCapability { capability, args };
                };
                changed = true;
                AgentAction::CallSkill {
                    skill: "git_basic".to_string(),
                    args: git_args,
                }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_readonly_git_run_cmd_to_git_basic");
    }
    rewritten
}

fn run_cmd_git_command_scope_matches_workspace(state: &AppState, args: &Value) -> bool {
    let Some(cwd) = args
        .get("cwd")
        .or_else(|| args.get("working_dir"))
        .or_else(|| args.get("workdir"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
    else {
        return true;
    };
    if !std::path::Path::new(cwd).is_absolute() {
        return false;
    }
    std::path::Path::new(cwd) == state.skill_rt.workspace_root.as_path()
}

fn git_basic_args_from_readonly_git_command(command: &str) -> Option<Value> {
    let command = command.trim();
    if command.is_empty() || command_has_shell_control_or_expansion(command) {
        return None;
    }
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    let executable = tokens.first()?.rsplit('/').next().unwrap_or_default();
    if executable != "git" {
        return None;
    }
    let subcommand = *tokens.get(1)?;
    if subcommand == "-C" {
        return None;
    }
    let args = &tokens[2..];
    let action = match subcommand {
        "status" if git_status_args_are_readonly(args) => "status",
        "remote" if git_remote_args_are_readonly(args) => "remote",
        "diff" if git_diff_args_are_changed_files(args) => "changed_files",
        _ => return None,
    };
    Some(serde_json::json!({ "action": action }))
}

fn git_status_args_are_readonly(args: &[&str]) -> bool {
    args.iter().all(|arg| {
        matches!(
            *arg,
            "--short"
                | "-s"
                | "--branch"
                | "-b"
                | "--porcelain"
                | "--porcelain=v1"
                | "--porcelain=v2"
        ) || arg.starts_with("--untracked-files")
    })
}

fn git_remote_args_are_readonly(args: &[&str]) -> bool {
    args.iter()
        .all(|arg| matches!(*arg, "-v" | "--verbose" | "show"))
}

fn git_diff_args_are_changed_files(args: &[&str]) -> bool {
    if args.is_empty() {
        return false;
    }
    let mut saw_name_only = false;
    for arg in args {
        match *arg {
            "--name-only" => saw_name_only = true,
            "HEAD" | "--cached" | "--staged" | "--" => {}
            value if value.starts_with("HEAD~") || value.starts_with("HEAD^") => {}
            _ => return false,
        }
    }
    saw_name_only
}

pub(super) fn rewrite_git_show_file_at_rev_capability_fs_reads(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["git"],
        &["show_file_at_rev"],
    ) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if matches!(skill.as_str(), "fs_basic" | "system_basic") =>
            {
                if let Some(args) = git_show_file_at_rev_args_from_fs_read(&args) {
                    AgentAction::CallSkill {
                        skill: "git_basic".to_string(),
                        args,
                    }
                } else {
                    AgentAction::CallSkill { skill, args }
                }
            }
            AgentAction::CallTool { tool, args }
                if matches!(tool.as_str(), "fs_basic" | "system_basic") =>
            {
                if let Some(args) = git_show_file_at_rev_args_from_fs_read(&args) {
                    AgentAction::CallSkill {
                        skill: "git_basic".to_string(),
                        args,
                    }
                } else {
                    AgentAction::CallTool { tool, args }
                }
            }
            other => other,
        })
        .collect()
}

fn git_show_file_at_rev_args_from_fs_read(args: &Value) -> Option<Value> {
    let Some(obj) = args.as_object() else {
        return None;
    };
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(action, "read_range" | "read_text_range") {
        return None;
    }
    let Some(path) = first_path_arg(obj) else {
        return None;
    };
    let target = obj
        .get("target")
        .or_else(|| obj.get("ref"))
        .or_else(|| obj.get("revision"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("HEAD");
    Some(serde_json::json!({
        "action": "show_file_at_rev",
        "target": target,
        "path": path,
    }))
}

fn first_path_arg(obj: &serde_json::Map<String, Value>) -> Option<String> {
    obj.get("path")
        .or_else(|| obj.get("resolved_path"))
        .or_else(|| obj.get("file"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| string_list_from_value(obj.get("paths")).into_iter().next())
}

pub(super) fn structured_scalar_read_action_for_target(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
    path: &str,
) -> Option<AgentAction> {
    let current_turn_sources = [Some(user_text), Some(route.resolved_intent.as_str())];
    let mut selectors = Vec::new();
    for text in current_turn_sources.iter().flatten() {
        for candidate in extract_dotted_field_selectors_for_structured_target(text) {
            push_unique_selector(&mut selectors, candidate);
        }
        for candidate in
            super::super::planning_structured_field_exact::extract_exact_structured_field_path_selectors(
                path, text,
            )
        {
            push_unique_selector(&mut selectors, candidate);
        }
        for candidate in extract_schema_identity_field_selectors(path, text) {
            push_unique_selector(&mut selectors, candidate);
        }
        for candidate in extract_schema_backed_field_selectors(path, text) {
            push_unique_selector(&mut selectors, candidate);
        }
    }
    selectors = prefer_non_locator_component_selectors(path, selectors);
    let field_path = selectors
        .into_iter()
        .find(|field| structured_field_selector_can_yield_scalar(state, path, field))?;
    Some(config_basic_read_field_action(path.to_string(), field_path))
}

pub(super) fn structured_field_selector_can_yield_scalar(
    state: &AppState,
    path: &str,
    field_path: &str,
) -> bool {
    if structured_field_path_resolves_scalar_value(path, field_path) {
        return true;
    }
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, path);
    resolve_cargo_workspace_package_fields(
        &state.skill_rt.workspace_root,
        &current,
        &[field_path.to_string()],
    )
    .is_some_and(|(target, fields)| {
        fields.len() == 1
            && structured_field_path_resolves_scalar_value(
                target.to_string_lossy().as_ref(),
                &fields[0],
            )
    })
}
