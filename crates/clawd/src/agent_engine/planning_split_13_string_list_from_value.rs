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
    let descending = obj
        .get("order")
        .or_else(|| obj.get("sort_order"))
        .or_else(|| obj.get("direction"))
        .and_then(Value::as_str)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "asc" | "ascending")
        })
        .unwrap_or(true);
    match sort_by.as_str() {
        "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc" | "name" => Some(sort_by),
        "mtime" | "modified" | "modified_ts" | "modified_time" => Some(if descending {
            "mtime_desc".to_string()
        } else {
            "mtime_asc".to_string()
        }),
        "size" | "size_bytes" | "bytes" => Some(if descending {
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
            if obj.contains_key("left_path") && obj.contains_key("right_path") {
                return args;
            }
            if !obj.contains_key("left_path") {
                if let Some(value) = obj
                    .remove("path1")
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
                    .or_else(|| obj.remove("right"))
                    .or_else(|| obj.remove("target_path"))
                    .or_else(|| obj.remove("second_path"))
                {
                    obj.insert("right_path".to_string(), value);
                }
            }
            if obj.contains_key("left_path") && obj.contains_key("right_path") {
                return args;
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

pub(super) fn git_repository_state_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState
    {
        return None;
    }
    let action = git_repository_state_action_from_text(user_text)?;
    Some(build_plan_result(
        goal,
        "deterministic:git_repository_state",
        PlanKind::Single,
        &[AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({ "action": action }),
        }],
    ))
}

pub(super) fn git_repository_state_action_from_text(user_text: &str) -> Option<&'static str> {
    if structural_token_present(user_text, "remote") {
        return Some("remote");
    }
    if structural_token_present(user_text, "status") {
        return Some("status");
    }
    if structural_token_present(user_text, "HEAD") {
        return Some("rev_parse");
    }
    None
}

pub(super) fn recent_scalar_current_workspace_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::RecentScalarEqualityCheck
        || route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !git_basic_available_for_plan(state)
    {
        return None;
    }
    let probe = AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "current_branch" }),
    };
    let AgentAction::CallSkill { skill, args } = &probe else {
        return None;
    };
    if crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        skill,
        args,
    )
    .is_some_and(|policy| !policy.is_allowed())
    {
        return None;
    }
    let actions = vec![
        probe,
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn recent_scalar_file_pair_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::RecentScalarEqualityCheck
    {
        return None;
    }

    let mut targets = structured_or_text_multi_file_targets(route, user_text);
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        targets.push(path.to_string());
    }
    targets.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

    let resolved_targets = targets
        .iter()
        .filter_map(|target| {
            resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, target)
        })
        .fold(Vec::<String>::new(), |mut out, path| {
            if !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&path))
            {
                out.push(path);
            }
            out
        });
    if resolved_targets.len() < 2 {
        return None;
    }

    let structured_reads = resolved_targets
        .iter()
        .filter(|path| path_has_structured_document_extension(path))
        .filter_map(|path| {
            structured_scalar_read_action_for_target(state, route, user_text, path.as_str())
        })
        .take(2)
        .collect::<Vec<_>>();
    let actions = if structured_reads.len() >= 2 {
        vec![structured_reads[0].clone(), structured_reads[1].clone()]
    } else {
        let mut structured_read: Option<AgentAction> = None;
        let mut text_query: Option<String> = None;
        for path in resolved_targets
            .iter()
            .filter(|path| path_has_structured_document_extension(path))
        {
            let selectors = structured_current_turn_field_selectors(route, user_text, Some(path));
            let Some(field_path) = selectors
                .into_iter()
                .find(|field| structured_field_selector_can_yield_scalar(state, path, field))
            else {
                continue;
            };
            text_query = structured_field_leaf_query(&field_path);
            structured_read = Some(config_basic_read_field_action(path.clone(), field_path));
            break;
        }
        let structured_read = structured_read?;
        let query = text_query?;
        let text_path = resolved_targets
            .iter()
            .find(|path| {
                !path_has_structured_document_extension(path) && Path::new(path).is_file()
            })?
            .clone();

        vec![
            structured_read,
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "grep_text",
                    "path": text_path,
                    "query": query,
                    "case_insensitive": true,
                    "max_results": 8,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ]
    };
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
        actions,
    );
    if actions
        .iter()
        .filter_map(|action| match action {
            AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
            AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
            _ => None,
        })
        .any(|(skill, args)| {
            !crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
        })
    {
        return None;
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
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

pub(super) fn structured_field_leaf_query(field_path: &str) -> Option<String> {
    field_path
        .split('.')
        .next_back()
        .map(str::trim)
        .filter(|leaf| schema_field_token_is_valid(leaf))
        .map(ToString::to_string)
}

pub(super) fn service_status_requests_system_basic_identity(
    route: &RouteResult,
    user_text: &str,
) -> bool {
    [user_text, route.resolved_intent.as_str()]
        .iter()
        .any(|text| {
            ["hostname", "host_name", "current_user", "whoami"]
                .iter()
                .any(|token| structural_token_present(text, token))
        })
}

pub(super) fn service_status_system_basic_info_plan(
    goal: &str,
    route: &RouteResult,
) -> Option<PlanResult> {
    let action = AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: serde_json::json!({"action":"info"}),
    };
    if let AgentAction::CallTool { tool: skill, args } = &action {
        if crate::contract_matrix::action_policy_for_output_contract(
            Some(&route.output_contract),
            skill,
            args,
        )
        .is_some_and(|policy| policy.is_allowed())
        {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_system_info",
                PlanKind::Single,
                &[action],
            ));
        }
    }
    None
}

pub(super) fn service_status_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || !route_requests_service_status(route)
    {
        return None;
    }
    if let Some(url) = service_status_url_locator(route, user_text) {
        let action = AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({
                "action": "get",
                "url": url,
            }),
        };
        if let AgentAction::CallSkill { skill, args } = &action {
            if crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
            {
                let actions = vec![
                    action,
                    AgentAction::SynthesizeAnswer {
                        evidence_refs: vec!["step_1".to_string()],
                    },
                    AgentAction::Respond {
                        content: "{{last_output}}".to_string(),
                    },
                ];
                return Some(build_plan_result(
                    goal,
                    "deterministic:service_status_http_get",
                    PlanKind::Single,
                    &actions,
                ));
            }
        }
    }
    if process_basic_available_for_plan(state)
        && !request_mentions_workspace_product(state, user_text)
    {
        if let Some(port) = first_port_filter_token(user_text) {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_port_list",
                PlanKind::Single,
                &[AgentAction::CallSkill {
                    skill: "process_basic".to_string(),
                    args: serde_json::json!({
                        "action": "port_list",
                        "filter": port,
                    }),
                }],
            ));
        }
        if let Some(filter) = process_status_filter_token(user_text) {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_process_list",
                PlanKind::Single,
                &[AgentAction::CallSkill {
                    skill: "process_basic".to_string(),
                    args: serde_json::json!({
                        "action": "ps",
                        "limit": 200,
                        "filter": filter,
                    }),
                }],
            ));
        }
    }
    if system_basic_available_for_plan(state)
        && service_status_requests_system_basic_identity(route, user_text)
    {
        if let Some(plan) = service_status_system_basic_info_plan(goal, route) {
            return Some(plan);
        }
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && health_check_available_for_plan(state)
    {
        let action = AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: serde_json::json!({}),
        };
        if let AgentAction::CallSkill { skill, args } = &action {
            if crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
            {
                return Some(build_plan_result(
                    goal,
                    "deterministic:service_status_scalar_health_check",
                    PlanKind::Single,
                    &[action],
                ));
            }
        }
    }
    if health_check_available_for_plan(state)
        && route_reason_has_marker(route, "execution_recipe_health_check_observation")
    {
        return Some(build_plan_result(
            goal,
            "deterministic:service_status_health_check_recipe",
            PlanKind::Single,
            &[AgentAction::CallSkill {
                skill: "health_check".to_string(),
                args: serde_json::json!({}),
            }],
        ));
    }
    if health_check_available_for_plan(state)
        && request_mentions_workspace_product(state, user_text)
    {
        return Some(build_plan_result(
            goal,
            "deterministic:service_status_health_check",
            PlanKind::Single,
            &[AgentAction::CallSkill {
                skill: "health_check".to_string(),
                args: serde_json::json!({}),
            }],
        ));
    }
    if health_check_available_for_plan(state) {
        let action = AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: serde_json::json!({}),
        };
        if let AgentAction::CallSkill { skill, args } = &action {
            if crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
            {
                return Some(build_plan_result(
                    goal,
                    "deterministic:service_status_health_check",
                    PlanKind::Single,
                    &[action],
                ));
            }
        }
    }
    if system_basic_available_for_plan(state) {
        if let Some(plan) = service_status_system_basic_info_plan(goal, route) {
            return Some(plan);
        }
    }
    if process_basic_available_for_plan(state) {
        if let Some(port) = first_port_filter_token(user_text) {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_port_list",
                PlanKind::Single,
                &[AgentAction::CallSkill {
                    skill: "process_basic".to_string(),
                    args: serde_json::json!({
                        "action": "port_list",
                        "filter": port,
                    }),
                }],
            ));
        }
        if let Some(filter) = process_status_filter_token(user_text) {
            return Some(build_plan_result(
                goal,
                "deterministic:service_status_process_list",
                PlanKind::Single,
                &[AgentAction::CallSkill {
                    skill: "process_basic".to_string(),
                    args: serde_json::json!({
                        "action": "ps",
                        "limit": 200,
                        "filter": filter,
                    }),
                }],
            ));
        }
    }
    None
}

pub(super) fn service_status_url_locator(route: &RouteResult, user_text: &str) -> Option<String> {
    [user_text, route.resolved_intent.as_str()]
        .into_iter()
        .filter_map(crate::intent::locator_extractor::extract_explicit_locator_for_fallback)
        .find(|locator| locator.locator_kind == crate::OutputLocatorKind::Url)
        .map(|locator| locator.locator_hint)
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (hint.starts_with("http://") || hint.starts_with("https://")).then(|| hint.to_string())
        })
}

pub(super) fn runtime_status_query_kind(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<&str> {
    turn_analysis
        .filter(|analysis| analysis.turn_type == Some(crate::intent_router::TurnType::StatusQuery))
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|patch| patch.get("runtime_status_query"))
        .and_then(Value::as_object)
        .and_then(|query| query.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
}

pub(super) fn runtime_status_query_system_basic_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "current_user" => Some("current_user"),
        "host_name" => Some("host_name"),
        "kernel_release" => Some("kernel_release"),
        "current_working_directory" | "current_process_cwd" | "process_cwd" => {
            Some("current_working_directory")
        }
        _ => None,
    }
}

pub(super) fn runtime_status_query_run_cmd_command(kind: &str) -> Option<&'static str> {
    match kind {
        "current_user" => Some("id -un"),
        "host_name" => Some("hostname"),
        "kernel_release" => Some("uname -r"),
        "current_working_directory" | "current_process_cwd" | "process_cwd" => Some("pwd"),
        _ => None,
    }
}
