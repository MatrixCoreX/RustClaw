use super::*;

pub(super) fn runtime_status_scalar_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<PlanResult> {
    let route = route_result?;
    let kind = runtime_status_query_kind(turn_analysis)?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !(route.output_contract_is_unclassified()
            || route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput))
        || route.output_contract.delivery_required
        || (!run_cmd_available_for_plan(state) && !system_basic_available_for_plan(state))
    {
        return None;
    }
    let action = if system_basic_available_for_plan(state) {
        let kind = runtime_status_query_system_basic_kind(kind)?;
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "runtime_status",
                "kind": kind,
            }),
        }
    } else {
        let command = runtime_status_query_run_cmd_command(kind)?;
        let mut args = serde_json::json!({
            "command": command,
            "cwd": state.skill_rt.workspace_root.display().to_string(),
        });
        args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args,
        }
    };
    if let AgentAction::CallTool { tool: skill, args } | AgentAction::CallSkill { skill, args } =
        &action
    {
        if !crate::contract_matrix::action_policy_for_route(Some(route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
        {
            return None;
        }
    }
    Some(build_plan_result(
        goal,
        "deterministic:runtime_status_scalar_system_basic",
        PlanKind::Single,
        &[action],
    ))
}

pub(super) fn route_reason_has_marker(route: &RouteResult, marker: &str) -> bool {
    route
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

pub(super) fn runtime_status_scalar_info_fallback_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<PlanResult> {
    let _ = (state, goal, route_result, loop_state, turn_analysis);
    None
}

pub(super) fn first_port_filter_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .find_map(port_filter_from_structural_token)
}

pub(super) fn process_status_contract_filter_token(route: &RouteResult) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() || !safe_process_status_filter_token(hint) {
        return None;
    }
    Some(hint.to_string())
}

pub(super) fn port_filter_from_structural_token(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|ch: char| {
        matches!(
            ch,
            ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}' | '"' | '\''
        )
    });
    let numeric = trimmed
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= 1024)
        .map(|port| port.to_string());
    if numeric.is_some() {
        return numeric;
    }
    let (_, port_part) = trimmed.rsplit_once(':')?;
    port_part
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .map(|port| port.to_string())
}

pub(super) fn safe_process_status_filter_token(token: &str) -> bool {
    token.len() >= 3
        && !token.chars().all(|ch| ch.is_ascii_digit())
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

pub(super) fn route_locator_hint_for_read_range_fill(
    route_result: Option<&RouteResult>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
    {
        return None;
    }
    if !matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return None;
    }
    route
        .output_contract
        .locator_hint
        .trim()
        .split('|')
        .next()
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(ToString::to_string)
}

pub(super) fn is_system_basic_read_range_missing_path(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("read_range"))
                && args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_none_or(|path| path.is_empty())
    )
}

pub(super) fn fill_missing_read_range_path_from_route_locator(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(locator_hint) = route_locator_hint_for_read_range_fill(route_result) else {
        return actions;
    };
    let missing_indices = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| is_system_basic_read_range_missing_path(action).then_some(idx))
        .collect::<Vec<_>>();
    if missing_indices.len() != 1 {
        return actions;
    }

    let mut rewritten = actions;
    let Some(action) = rewritten.get_mut(missing_indices[0]) else {
        return rewritten;
    };
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert("path".to_string(), Value::String(locator_hint.clone()));
            }
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {}
    }
    info!(
        "plan_fill_missing_read_range_path_from_route_locator idx={} path={}",
        missing_indices[0], locator_hint
    );
    rewritten
}

pub(super) fn bool_arg_any(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| obj.get(*key).and_then(Value::as_bool).unwrap_or(false))
}

pub(super) fn structured_directory_filter_requested(obj: &serde_json::Map<String, Value>) -> bool {
    bool_arg_any(
        obj,
        &[
            "dirs_only",
            "directories_only",
            "directory_only",
            "folders_only",
        ],
    ) || string_arg_any_matches(
        obj,
        &[
            "kind_filter",
            "filter_kind",
            "kind",
            "entry_type",
            "target_kind",
        ],
        &[
            "dir",
            "dirs",
            "directory",
            "directories",
            "folder",
            "folders",
        ],
    )
}

pub(super) fn structured_file_filter_requested(obj: &serde_json::Map<String, Value>) -> bool {
    bool_arg_any(obj, &["files_only", "file_only"])
        || string_arg_any_matches(
            obj,
            &[
                "kind_filter",
                "filter_kind",
                "kind",
                "entry_type",
                "target_kind",
            ],
            &["file", "files"],
        )
}

pub(super) fn string_arg_any_matches(
    obj: &serde_json::Map<String, Value>,
    keys: &[&str],
    values: &[&str],
) -> bool {
    keys.iter().any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|raw| values.iter().any(|value| raw.eq_ignore_ascii_case(value)))
    })
}

pub(super) fn normalize_schema_token(raw: &str) -> String {
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

pub(super) fn list_dir_args_need_inventory_dir(
    state: &AppState,
    _route_result: Option<&RouteResult>,
    obj: &serde_json::Map<String, Value>,
) -> bool {
    let Some(manifest) = state.skill_manifest("list_dir") else {
        return obj.contains_key("dirs_only")
            || obj.contains_key("directories_only")
            || obj.contains_key("directory_only")
            || obj.contains_key("folders_only")
            || obj.contains_key("files_only")
            || obj.contains_key("kind_filter")
            || obj.contains_key("kind")
            || obj.contains_key("entry_type")
            || obj.contains_key("include_hidden")
            || obj.contains_key("sort_by")
            || obj.contains_key("ext_filter")
            || obj.contains_key("extension")
            || obj.contains_key("extensions");
    };
    obj.keys().any(|key| {
        let normalized = normalize_schema_token(key);
        manifest
            .runtime_rewrite_arg_keys
            .iter()
            .any(|candidate| candidate == &normalized)
    })
}

pub(super) fn list_dir_runtime_mapping_from_registry(
    state: &AppState,
) -> (String, String, Option<serde_json::Value>) {
    let Some(manifest) = state.skill_manifest("list_dir") else {
        return (
            "system_basic".to_string(),
            "inventory_dir".to_string(),
            Some(serde_json::json!({"names_only": true})),
        );
    };
    let runtime_skill = manifest
        .runtime_skill
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("system_basic")
        .to_string();
    let runtime_action = manifest
        .runtime_action
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("inventory_dir")
        .to_string();
    (runtime_skill, runtime_action, manifest.runtime_default_args)
}

pub(super) fn merge_default_args(
    obj: &mut serde_json::Map<String, Value>,
    defaults: Option<Value>,
) {
    let Some(defaults) = defaults.and_then(|value| value.as_object().cloned()) else {
        return;
    };
    for (key, value) in defaults {
        obj.entry(key).or_insert(value);
    }
}

pub(super) fn inventory_dir_args_from_list_dir_args(
    state: &AppState,
    route_result: Option<&RouteResult>,
    args: Value,
) -> Option<(String, Value)> {
    let mut obj = args.as_object()?.clone();
    if !list_dir_args_need_inventory_dir(state, route_result, &obj) {
        return None;
    }
    let (runtime_skill, runtime_action, default_args) =
        list_dir_runtime_mapping_from_registry(state);
    merge_default_args(&mut obj, default_args);
    normalize_path_alias_to_path(
        &mut obj,
        &["dir_path", "directory_path", "directory", "dir"],
    );
    obj.insert("action".to_string(), Value::String(runtime_action));
    let route_is_directory_names = route_result.is_some_and(|route| {
        route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryNames)
    });
    let route_is_file_names = route_result
        .is_some_and(|route| route.output_contract_marker_is(crate::OutputSemanticKind::FileNames));
    let directory_filter_requested = structured_directory_filter_requested(&obj);
    let file_filter_requested = structured_file_filter_requested(&obj);
    let mut dirs_only =
        directory_filter_requested || (route_is_directory_names && file_filter_requested);
    let mut files_only = route_is_file_names || file_filter_requested;
    if dirs_only {
        files_only = false;
    } else if files_only {
        dirs_only = false;
    }
    obj.insert("dirs_only".to_string(), Value::Bool(dirs_only));
    obj.insert("files_only".to_string(), Value::Bool(files_only));
    if dirs_only || files_only || route_is_directory_names || route_is_file_names {
        obj.insert("names_only".to_string(), Value::Bool(true));
    } else {
        obj.entry("names_only".to_string())
            .or_insert_with(|| Value::Bool(true));
    }
    if route_is_directory_names || route_is_file_names {
        obj.insert("include_hidden".to_string(), Value::Bool(false));
    }
    for key in [
        "directories_only",
        "directory_only",
        "folders_only",
        "file_only",
        "kind_filter",
        "kind",
        "entry_type",
        "extension",
        "extensions",
    ] {
        if key == "extension" || key == "extensions" {
            if !obj.contains_key("ext_filter") {
                if let Some(value) = obj.remove(key) {
                    obj.insert("ext_filter".to_string(), value);
                }
                continue;
            }
        }
        obj.remove(key);
    }
    Some((runtime_skill, Value::Object(obj)))
}

pub(super) fn rewrite_filtered_list_dir_to_inventory_dir(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("list_dir") => {
                if let Some((runtime_skill, args)) =
                    inventory_dir_args_from_list_dir_args(state, route_result, args.clone())
                {
                    info!(
                        "plan_rewrite_list_dir_to_inventory_dir runtime_skill={}",
                        runtime_skill
                    );
                    AgentAction::CallSkill {
                        skill: runtime_skill,
                        args,
                    }
                } else {
                    AgentAction::CallSkill { skill, args }
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("list_dir") => {
                if let Some((runtime_skill, args)) =
                    inventory_dir_args_from_list_dir_args(state, route_result, args.clone())
                {
                    info!(
                        "plan_rewrite_list_dir_to_inventory_dir runtime_skill={}",
                        runtime_skill
                    );
                    AgentAction::CallTool {
                        tool: runtime_skill,
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

pub(super) fn normalize_doc_parse_args(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(action_name.as_str(), "parse_doc" | "parse") {
        obj.insert("action".to_string(), Value::String("parse_doc".to_string()));
    }
    normalize_path_alias_to_path(obj, &["file", "file_path", "document", "document_path"]);
    args
}

pub(super) fn normalize_doc_parse_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "doc_parse" => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_doc_parse_args(args),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_transform_args(mut args: Value) -> Value {
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
    if !matches!(action_name.as_str(), "transform_data") {
        obj.insert(
            "action".to_string(),
            Value::String("transform_data".to_string()),
        );
        info!(
            "plan_normalize_transform_action_alias action={} normalized=transform_data",
            action_name
        );
    }
    normalize_arg_alias(obj, "data", &["records", "items", "rows", "array"]);
    if !obj.contains_key("ops") {
        let sort_by = obj
            .remove("sort_by")
            .or_else(|| obj.remove("sort_field"))
            .or_else(|| obj.remove("order_by"))
            .or_else(|| obj.remove("by"));
        if let Some(sort_by) = sort_by.filter(has_non_empty_json_value) {
            let mut op = serde_json::Map::new();
            op.insert("op".to_string(), Value::String("sort".to_string()));
            op.insert("by".to_string(), sort_by);
            if let Some(order) = obj
                .remove("order")
                .or_else(|| obj.remove("sort_order"))
                .filter(has_non_empty_json_value)
            {
                op.insert("order".to_string(), order);
            }
            obj.insert("ops".to_string(), Value::Array(vec![Value::Object(op)]));
            info!("plan_normalize_transform_sort_alias_to_ops");
        }
    }
    normalize_transform_ops(obj);
    args
}

pub(super) fn normalize_transform_ops(obj: &mut serde_json::Map<String, Value>) {
    let Some(ops) = obj.get_mut("ops").and_then(Value::as_array_mut) else {
        return;
    };
    for op in ops {
        let Some(op_obj) = op.as_object_mut() else {
            continue;
        };
        let op_name = op_obj
            .get("op")
            .and_then(Value::as_str)
            .map(|name| name.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if op_name == "filter" {
            normalize_transform_filter_op(op_obj);
        }
    }
}

pub(super) fn normalize_transform_filter_op(op: &mut serde_json::Map<String, Value>) {
    let Some(where_obj) = op.get("where").and_then(Value::as_object).cloned() else {
        return;
    };
    if !op.contains_key("field") {
        if let Some(field) = where_obj
            .get("field")
            .or_else(|| where_obj.get("path"))
            .filter(|value| has_non_empty_json_value(value))
            .cloned()
        {
            op.insert("field".to_string(), field);
        }
    }
    if !op.contains_key("path") {
        if let Some(path) = where_obj
            .get("path")
            .filter(|value| has_non_empty_json_value(value))
            .cloned()
        {
            op.insert("path".to_string(), path);
        }
    }
    if !op.contains_key("cmp") {
        if let Some(cmp) = where_obj
            .get("cmp")
            .or_else(|| where_obj.get("operator"))
            .and_then(Value::as_str)
            .and_then(normalize_transform_cmp_alias)
        {
            op.insert("cmp".to_string(), Value::String(cmp.to_string()));
        } else if let Some(cmp) = transform_where_comparator_key(&where_obj) {
            op.insert("cmp".to_string(), Value::String(cmp.to_string()));
        }
    }
    if !op.contains_key("value") {
        if let Some(value) = where_obj
            .get("value")
            .filter(|value| has_non_empty_json_value(value))
            .cloned()
        {
            op.insert("value".to_string(), value);
        } else if let Some(cmp_key) = transform_where_comparator_key(&where_obj) {
            if let Some(value) = where_obj.get(cmp_key).cloned() {
                op.insert("value".to_string(), value);
            }
        }
    }
}

pub(super) fn transform_where_comparator_key(
    where_obj: &serde_json::Map<String, Value>,
) -> Option<&str> {
    where_obj
        .keys()
        .filter_map(|key| normalize_transform_cmp_alias(key).map(|cmp| (key.as_str(), cmp)))
        .find(|(key, _)| !matches!(*key, "field" | "path" | "cmp" | "operator" | "value" | "op"))
        .map(|(key, _)| key)
}

pub(super) fn normalize_transform_cmp_alias(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "eq" | "equal" | "equals" => Some("eq"),
        "ne" | "neq" | "not_eq" | "not_equals" => Some("ne"),
        "gt" => Some("gt"),
        "gte" | "ge" => Some("gte"),
        "lt" => Some("lt"),
        "lte" | "le" => Some("lte"),
        "contains" => Some("contains"),
        "in" => Some("in"),
        "exists" => Some("exists"),
        _ => None,
    }
}

pub(super) fn normalize_transform_schema_aliases(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("transform") => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_transform_args(args),
                }
            }
            AgentAction::CallTool { tool, args } if tool.eq_ignore_ascii_case("transform") => {
                AgentAction::CallTool {
                    tool,
                    args: normalize_transform_args(args),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_archive_basic_args(
    mut args: Value,
    route_result: Option<&RouteResult>,
) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    let mut action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(action_name.as_str(), "list" | "read" | "pack" | "unpack")
        && unknown_archive_action_can_normalize_to_list(obj, route_result)
    {
        obj.insert("action".to_string(), Value::String("list".to_string()));
        action_name = "list".to_string();
    }
    match action_name.as_str() {
        "pack" => {
            normalize_arg_alias(
                obj,
                "source",
                &["source_path", "src", "input", "input_path"],
            );
            normalize_arg_alias(
                obj,
                "archive",
                &[
                    "output",
                    "archive_path",
                    "target",
                    "destination",
                    "output_path",
                    "target_path",
                    "destination_path",
                ],
            );
            if let Some((source, archive)) = route_result.and_then(archive_pack_pair_for_route) {
                obj.entry("source".to_string())
                    .or_insert_with(|| Value::String(source));
                obj.entry("archive".to_string())
                    .or_insert_with(|| Value::String(archive));
            }
            if !obj.contains_key("format") {
                if let Some(archive) = obj
                    .get("archive")
                    .and_then(Value::as_str)
                    .filter(|archive| is_supported_archive_path(archive))
                {
                    obj.insert(
                        "format".to_string(),
                        Value::String(archive_format_for_path(archive).to_string()),
                    );
                }
            }
        }
        "unpack" => {
            normalize_arg_alias(
                obj,
                "archive",
                &["archive_path", "path", "input", "input_path"],
            );
            normalize_arg_alias(
                obj,
                "dest",
                &["dest_path", "destination", "destination_path", "output_dir"],
            );
        }
        "list" => {
            normalize_arg_alias(
                obj,
                "archive",
                &["archive_path", "path", "input", "input_path"],
            );
        }
        "read" => {
            normalize_arg_alias(
                obj,
                "archive",
                &["archive_path", "path", "input", "input_path"],
            );
            normalize_arg_alias(
                obj,
                "member",
                &[
                    "entry",
                    "entry_path",
                    "member_path",
                    "file",
                    "file_path",
                    "path_inside_archive",
                ],
            );
        }
        _ => {}
    }
    args
}

pub(super) fn unknown_archive_action_can_normalize_to_list(
    obj: &serde_json::Map<String, Value>,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if archive_args_have_pack_or_unpack_shape(obj) {
        return false;
    }
    let archive = archive_arg_candidate_from_obj(obj).or_else(|| {
        let hint = route.output_contract.locator_hint.trim();
        (!hint.is_empty()).then(|| hint.to_string())
    });
    let Some(archive) = archive.filter(|archive| is_supported_archive_path(archive)) else {
        return false;
    };
    if archive_args_have_entry_selector(obj) {
        return true;
    }
    archive_list_auto_locator_target_path(Some(route), Some(&archive)).is_some()
}

pub(super) fn archive_args_have_entry_selector(obj: &serde_json::Map<String, Value>) -> bool {
    [
        "entry",
        "entry_path",
        "member",
        "member_path",
        "file",
        "file_path",
        "name",
        "path_inside_archive",
    ]
    .iter()
    .any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|value| !is_supported_archive_path(value))
    }) || obj
        .get("entries")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some_and(|value| !is_supported_archive_path(value))
            })
        })
        || obj
            .get("members")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_some_and(|value| !is_supported_archive_path(value))
                })
            })
}

pub(super) fn archive_args_have_pack_or_unpack_shape(obj: &serde_json::Map<String, Value>) -> bool {
    [
        "source",
        "source_path",
        "src",
        "dest",
        "dest_path",
        "destination",
        "destination_path",
        "output",
        "output_path",
    ]
    .iter()
    .any(|key| obj.contains_key(*key))
}

pub(super) fn archive_arg_candidate_from_obj(
    obj: &serde_json::Map<String, Value>,
) -> Option<String> {
    ["archive", "archive_path", "path", "input", "input_path"]
        .iter()
        .find_map(|key| {
            obj.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
}

pub(super) fn normalize_archive_basic_schema_aliases(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "archive_basic" => {
                AgentAction::CallSkill {
                    skill,
                    args: normalize_archive_basic_args(args, route_result),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn rewrite_archive_basic_short_archive_to_active_bound_target(
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let targets = active_bound_archive_targets_from_plan_context(plan_context);
    if targets.is_empty() {
        return actions;
    }
    actions
        .into_iter()
        .enumerate()
        .map(|(idx, action)| {
            let AgentAction::CallSkill { skill, mut args } = action else {
                return action;
            };
            if !skill.eq_ignore_ascii_case("archive_basic") {
                return AgentAction::CallSkill { skill, args };
            }
            let Some(obj) = args.as_object_mut() else {
                return AgentAction::CallSkill { skill, args };
            };
            let action_name = obj
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !action_name.eq_ignore_ascii_case("list") {
                return AgentAction::CallSkill { skill, args };
            }
            let Some(current_archive) = obj
                .get("archive")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
            else {
                return AgentAction::CallSkill { skill, args };
            };
            let Some(target) = targets
                .iter()
                .find(|target| short_locator_matches_target_basename(&current_archive, target))
            else {
                return AgentAction::CallSkill { skill, args };
            };
            obj.insert("archive".to_string(), Value::String(target.clone()));
            info!(
                "plan_rewrite_archive_short_archive_to_active_bound_target idx={} from={} to={}",
                idx, current_archive, target
            );
            AgentAction::CallSkill { skill, args }
        })
        .collect()
}

pub(super) fn active_bound_archive_targets_from_plan_context(
    plan_context: Option<&str>,
) -> Vec<String> {
    let Some(plan_context) = plan_context else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for line in plan_context.lines() {
        let trimmed = line.trim_start();
        let target = ["followup_bound_target:", "observed_bound_target:"]
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
            .map(str::trim)
            .filter(|target| is_supported_archive_path(target));
        let Some(target) = target else {
            continue;
        };
        if !targets
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(target))
        {
            targets.push(target.to_string());
        }
    }
    targets
}

pub(super) fn short_locator_matches_target_basename(locator: &str, target: &str) -> bool {
    let locator = trim_archive_bound_locator_token(locator);
    if locator.is_empty()
        || locator.contains('/')
        || locator.contains('\\')
        || Path::new(&locator).is_absolute()
        || Path::new(&locator).components().count() != 1
        || !is_supported_archive_path(&locator)
    {
        return false;
    }
    Path::new(target)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(&locator))
        .unwrap_or(false)
}

pub(super) fn trim_archive_bound_locator_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | ','
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        })
        .to_string()
}

pub(super) fn strip_directory_read_range_after_inventory_dir(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut inventory_paths: Vec<String> = Vec::new();
    let mut old_to_new: Vec<Option<usize>> = vec![None; actions.len() + 1];
    let mut stripped = Vec::new();
    let mut stripped_indices = Vec::new();

    for (idx, action) in actions.into_iter().enumerate() {
        let old_idx = idx + 1;
        if let Some((action_name, path)) = system_basic_action_path(&action) {
            if action_name == "read_range" && inventory_paths.iter().any(|known| known == &path) {
                stripped_indices.push(old_idx);
                continue;
            }
            if action_name == "inventory_dir" && !inventory_paths.iter().any(|known| known == &path)
            {
                inventory_paths.push(path);
            }
        }
        old_to_new[old_idx] = Some(stripped.len() + 1);
        stripped.push(action);
    }

    if stripped_indices.is_empty() {
        return stripped;
    }

    for action in &mut stripped {
        if let AgentAction::SynthesizeAnswer { evidence_refs } = action {
            *evidence_refs = rewrite_evidence_refs_after_step_strip(evidence_refs, &old_to_new);
        }
    }

    info!(
        "plan_strip_directory_read_range_after_inventory_dir stripped_steps={}",
        stripped_indices
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    stripped
}

pub(super) fn system_basic_action_path(action: &AgentAction) -> Option<(String, String)> {
    let AgentAction::CallSkill { skill, args } = action else {
        return None;
    };
    if skill != "system_basic" {
        return None;
    }
    let obj = args.as_object()?;
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    let raw_path = if action_name == "inventory_dir" {
        obj.get("path").and_then(Value::as_str).unwrap_or(".")
    } else {
        obj.get("path").and_then(Value::as_str)?
    };
    let path = normalize_plan_path(raw_path)?;
    Some((action_name, path))
}

pub(super) fn normalize_plan_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_trailing_slashes = trimmed.trim_end_matches('/');
    if without_trailing_slashes.is_empty() {
        Some("/".to_string())
    } else {
        Some(without_trailing_slashes.to_string())
    }
}
