use super::*;

pub(super) fn system_basic_action_path_and_args(
    action: &AgentAction,
) -> Option<(String, String, &serde_json::Map<String, Value>)> {
    let AgentAction::CallSkill { skill, args } = action else {
        return None;
    };
    if skill != "system_basic" {
        return None;
    }
    let obj = args.as_object()?;
    let (action_name, path) = system_basic_action_path(action)?;
    Some((action_name, path, obj))
}

pub(super) fn strip_file_lines_count_before_tail_read_range(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut stripped = 0usize;
    let mut idx = 0usize;
    while idx < actions.len() {
        let should_strip = if let Some((action_name, path, _)) =
            system_basic_action_path_and_args(&actions[idx])
        {
            action_name == "file_lines_count"
                && actions.get(idx + 1).is_some_and(|next| {
                    system_basic_action_path_and_args(next).is_some_and(
                        |(next_action, next_path, next_args)| {
                            next_action == "read_range"
                                && next_path == path
                                && next_args
                                    .get("mode")
                                    .and_then(Value::as_str)
                                    .is_some_and(|mode| mode.eq_ignore_ascii_case("tail"))
                        },
                    )
                })
        } else {
            false
        };
        if should_strip {
            stripped += 1;
        } else {
            rewritten.push(actions[idx].clone());
        }
        idx += 1;
    }
    if stripped > 0 {
        info!(
            "plan_strip_file_lines_count_before_tail_read_range stripped_steps={}",
            stripped
        );
    }
    rewritten
}

pub(super) fn rewrite_evidence_refs_after_step_strip(
    refs: &[String],
    old_to_new: &[Option<usize>],
) -> Vec<String> {
    let mut rewritten = Vec::new();
    for evidence_ref in refs {
        let Some(replacement) =
            rewrite_single_evidence_ref_after_step_strip(evidence_ref, old_to_new)
        else {
            continue;
        };
        if !rewritten.iter().any(|existing| existing == &replacement) {
            rewritten.push(replacement);
        }
    }
    if rewritten.is_empty() {
        rewritten.push("last_output".to_string());
    }
    rewritten
}

pub(super) fn rewrite_single_evidence_ref_after_step_strip(
    evidence_ref: &str,
    old_to_new: &[Option<usize>],
) -> Option<String> {
    let trimmed = evidence_ref.trim();
    if let Some(step_idx) = trimmed
        .strip_prefix("step_")
        .and_then(|value| value.parse::<usize>().ok())
    {
        return old_to_new
            .get(step_idx)
            .and_then(|value| *value)
            .map(|new_idx| format!("step_{new_idx}"));
    }
    if let Some(step_idx) = trimmed
        .strip_prefix('s')
        .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok())
    {
        return old_to_new
            .get(step_idx)
            .and_then(|value| *value)
            .map(|new_idx| format!("s{new_idx}"));
    }
    Some(evidence_ref.to_string())
}

pub(super) fn enforce_output_contract_tool_args(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };

    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("system_basic") =>
                {
                    if rewrite_inventory_ext_filter_action_to_fs_basic(
                        route,
                        user_text,
                        original_user_text,
                        skill,
                        args,
                    ) {
                        return action;
                    }
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    let action_name = obj
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_string();
                    let action_name_lower = action_name.to_ascii_lowercase();
                    if action_name_lower == "inventory_dir" {
                        enforce_file_names_inventory_args(route, obj);
                        enforce_directory_names_inventory_args(route, obj);
                        enforce_directory_entry_groups_inventory_args(route, obj);
                        enforce_general_directory_inventory_args(route, obj);
                        enforce_strict_directory_metadata_inventory_args(route, obj);
                    }
                    if !route
                        .output_contract_marker_is(crate::OutputSemanticKind::HiddenEntriesCheck)
                    {
                        return action;
                    }
                    if matches!(
                        action_name_lower.as_str(),
                        "inventory_dir" | "count_inventory" | "workspace_glance"
                    ) {
                        obj.insert("include_hidden".to_string(), Value::Bool(true));
                        info!(
                            "plan_contract_enforce_hidden_inventory action={}",
                            action_name
                        );
                    }
                }
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_search") =>
                {
                    enforce_fs_search_path_output_args(route, args);
                }
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_basic") =>
                {
                    if rewrite_inventory_ext_filter_action_to_fs_basic(
                        route,
                        user_text,
                        original_user_text,
                        skill,
                        args,
                    ) {
                        return action;
                    }
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    let action_name = obj
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if rewrite_file_paths_grep_text_to_find_entries(route, obj) {
                        enforce_file_paths_result_limit(
                            route,
                            user_text,
                            original_user_text,
                            "find_entries",
                            obj,
                        );
                        return action;
                    }
                    if matches!(
                        action_name.as_str(),
                        "find_entries" | "find_ext" | "list_dir"
                    ) {
                        enforce_file_names_inventory_args(route, obj);
                        enforce_directory_names_inventory_args(route, obj);
                        enforce_directory_entry_groups_inventory_args(route, obj);
                        enforce_file_paths_result_limit(
                            route,
                            user_text,
                            original_user_text,
                            &action_name,
                            obj,
                        );
                    }
                    if action_name == "list_dir" {
                        enforce_strict_directory_metadata_inventory_args(route, obj);
                    }
                }
                _ => {}
            }
            action
        })
        .collect()
}

pub(super) fn rewrite_file_paths_grep_text_to_find_entries(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths) {
        return false;
    }
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !action_name.eq_ignore_ascii_case("grep_text") {
        return false;
    }
    if crate::contract_matrix::action_policy_for_route(
        Some(route),
        "fs_basic",
        &Value::Object(obj.clone()),
    )
    .is_some_and(|policy| policy.is_allowed())
    {
        return false;
    }
    let Some(query) = obj
        .get("query")
        .or_else(|| obj.get("keyword"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    else {
        return false;
    };
    let ext = first_ext_filter_value(obj).or_else(|| {
        obj.get("pattern")
            .and_then(Value::as_str)
            .and_then(extension_from_globish_pattern)
    });
    obj.insert(
        "action".to_string(),
        Value::String("find_entries".to_string()),
    );
    obj.insert("pattern".to_string(), Value::String(query));
    obj.insert("target_kind".to_string(), Value::String("file".to_string()));
    obj.entry("recursive".to_string())
        .or_insert(Value::Bool(true));
    if let Some(ext) = ext {
        obj.insert("ext".to_string(), Value::String(ext));
    }
    obj.remove("query");
    obj.remove("keyword");
    info!("plan_contract_rewrite_file_paths_grep_text_to_find_entries");
    true
}

pub(super) fn prune_file_paths_contract_disallowed_actions(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths)
        || route.output_contract.delivery_required
    {
        return actions;
    }
    let has_allowed_executable = actions.iter().any(|action| {
        matches!(
            file_paths_contract_executable_action_allowed(action),
            Some(true)
        )
    });
    if !has_allowed_executable {
        return actions;
    }

    let keep_flags = actions
        .iter()
        .map(|action| {
            !matches!(
                file_paths_contract_executable_action_allowed(action),
                Some(false)
            )
        })
        .collect::<Vec<_>>();
    if keep_flags.iter().all(|keep| *keep) {
        return actions;
    }

    let mut old_to_new = vec![None; actions.len() + 1];
    let mut next_idx = 1usize;
    for (old_idx, keep) in keep_flags.iter().enumerate() {
        if *keep {
            old_to_new[old_idx + 1] = Some(next_idx);
            next_idx += 1;
        }
    }

    let stripped = keep_flags.iter().filter(|keep| !**keep).count();
    let rewritten = actions
        .into_iter()
        .zip(keep_flags)
        .filter_map(|(action, keep)| {
            if !keep {
                return None;
            }
            Some(match action {
                AgentAction::SynthesizeAnswer { evidence_refs } => AgentAction::SynthesizeAnswer {
                    evidence_refs: rewrite_evidence_refs_after_step_strip(
                        &evidence_refs,
                        &old_to_new,
                    ),
                },
                other => other,
            })
        })
        .collect::<Vec<_>>();
    info!(
        "plan_contract_prune_file_paths_disallowed_actions stripped_steps={}",
        stripped
    );
    rewritten
}

pub(super) fn file_paths_contract_executable_action_allowed(action: &AgentAction) -> Option<bool> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { .. } => return Some(false),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => return None,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let skill = skill.trim().to_ascii_lowercase();
    let action_name = action_name.to_ascii_lowercase();
    Some(match skill.as_str() {
        "fs_basic" => matches!(
            action_name.as_str(),
            "find_entries" | "find_ext" | "grep_text" | "list_dir"
        ),
        "fs_search" => matches!(
            action_name.as_str(),
            "find_name" | "find_ext" | "find_path" | "find_entries" | "grep_text"
        ),
        _ => false,
    })
}

pub(super) fn structural_extension_filter_from_text(text: &str) -> Option<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '*')))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .find_map(|token| {
            extension_from_globish_pattern(token)
                .or_else(|| extension_from_bare_extension_token(token))
        })
}

pub(super) fn extension_from_bare_extension_token(text: &str) -> Option<String> {
    let cleaned = text
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if cleaned.is_empty()
        || cleaned.contains(['*', '?', '/', '\\', '.'])
        || !cleaned
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    // These are language-neutral file extension tokens, not user phrase rules.
    const STRUCTURAL_EXTENSION_TOKENS: &[&str] = &[
        "bash", "cfg", "conf", "css", "csv", "env", "html", "ini", "js", "json", "jsonl", "jsx",
        "lock", "log", "md", "mjs", "py", "rs", "scss", "sh", "sql", "toml", "ts", "tsx", "txt",
        "xml", "yaml", "yml", "zsh",
    ];
    STRUCTURAL_EXTENSION_TOKENS
        .contains(&cleaned.as_str())
        .then_some(cleaned)
}

pub(super) fn structural_extension_filter_for_directory_inventory(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
) -> Option<String> {
    let hint_texts = [
        Some(route.resolved_intent.as_str()),
        Some(user_text),
        original_user_text,
        Some(route.route_reason.as_str()),
    ];
    if let Some(extension) = hint_texts
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .find_map(contract_hint_selector_extension)
    {
        return Some(extension);
    }

    [
        Some(user_text),
        original_user_text,
        Some(route.resolved_intent.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|text| !text.is_empty())
    .find_map(structural_extension_filter_from_text)
}

pub(super) fn route_allows_structural_extension_inventory_filter(route: &RouteResult) -> bool {
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && (route.output_contract_is_unclassified()
            || [
                crate::OutputSemanticKind::QuantityComparison,
                crate::OutputSemanticKind::DirectoryPurposeSummary,
                crate::OutputSemanticKind::FileNames,
                crate::OutputSemanticKind::FilePaths,
                crate::OutputSemanticKind::DirectoryEntryGroups,
            ]
            .iter()
            .any(|kind| route.output_contract_marker_is(*kind)))
}

pub(super) fn inject_structural_extension_filter_for_directory_inventory(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_allows_structural_extension_inventory_filter(route) {
        return actions;
    }
    let Some(ext) =
        structural_extension_filter_for_directory_inventory(route, user_text, original_user_text)
    else {
        return actions;
    };
    let mut changed = false;
    let actions = actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_basic")
                        || skill.eq_ignore_ascii_case("system_basic") =>
                {
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    let action_name = obj
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if matches!(action_name.as_str(), "list_dir" | "inventory_dir") {
                        if !obj.get("ext_filter").is_some_and(has_non_empty_json_value)
                            && !obj.get("extension").is_some_and(has_non_empty_json_value)
                            && !obj.get("extensions").is_some_and(has_non_empty_json_value)
                        {
                            obj.insert(
                                "ext_filter".to_string(),
                                Value::Array(vec![Value::String(ext.clone())]),
                            );
                            obj.insert("files_only".to_string(), Value::Bool(true));
                            obj.insert("dirs_only".to_string(), Value::Bool(false));
                            if route.output_contract_marker_is(
                                crate::OutputSemanticKind::QuantityComparison,
                            ) {
                                obj.insert(
                                    "max_entries".to_string(),
                                    Value::Number(serde_json::Number::from(1000)),
                                );
                            }
                            changed = true;
                        }
                    } else if matches!(action_name.as_str(), "find_entries" | "find_ext") {
                        if !obj.get("ext").is_some_and(has_non_empty_json_value)
                            && !obj.get("ext_filter").is_some_and(has_non_empty_json_value)
                        {
                            obj.insert("ext".to_string(), Value::String(ext.clone()));
                            obj.insert(
                                "target_kind".to_string(),
                                Value::String("file".to_string()),
                            );
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
            action
        })
        .collect();
    if changed {
        info!(
            "plan_inject_structural_extension_inventory_filter ext={}",
            crate::truncate_for_log(&ext)
        );
    }
    actions
}

pub(super) fn route_requests_general_directory_inventory(route: &RouteResult) -> bool {
    if route.output_contract.delivery_required {
        return false;
    }
    if route.output_contract.delivery_intent == crate::OutputDeliveryIntent::DirectoryLookup {
        return true;
    }
    route.output_contract.response_shape == crate::OutputResponseShape::Free
        && (route.output_contract_is_unclassified()
            || route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryPurposeSummary))
}

pub(super) fn inventory_dir_has_filter_args(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("ext_filter").is_some_and(has_non_empty_json_value)
        || obj.get("extension").is_some_and(has_non_empty_json_value)
        || obj.get("extensions").is_some_and(has_non_empty_json_value)
}

pub(super) fn enforce_general_directory_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route_requests_general_directory_inventory(route) || inventory_dir_has_filter_args(obj) {
        return;
    }
    obj.insert("files_only".to_string(), Value::Bool(false));
    obj.insert("dirs_only".to_string(), Value::Bool(false));
    if route.output_contract_marker_is(crate::OutputSemanticKind::FileNames) {
        obj.insert("names_only".to_string(), Value::Bool(true));
        info!("plan_contract_enforce_directory_entry_names_inventory");
        return;
    }
    obj.insert("names_only".to_string(), Value::Bool(false));
    info!("plan_contract_enforce_general_directory_inventory");
}

pub(super) fn route_requires_strict_directory_metadata_inventory(route: &RouteResult) -> bool {
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract_is_unclassified()
}

pub(super) fn enforce_strict_directory_metadata_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route_requires_strict_directory_metadata_inventory(route) {
        return;
    }
    obj.insert("names_only".to_string(), Value::Bool(false));
    obj.entry("max_entries".to_string())
        .or_insert_with(|| Value::Number(serde_json::Number::from(1000)));
    info!("plan_contract_enforce_strict_directory_metadata_inventory");
}

pub(super) fn enforce_directory_names_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryNames) {
        return;
    }
    let include_hidden = route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden
        .unwrap_or(false);
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if matches!(action_name, "find_entries" | "find_ext")
        && (structured_file_filter_requested(obj)
            || obj.get("extension").is_some_and(has_non_empty_json_value)
            || obj.get("ext").is_some_and(has_non_empty_json_value)
            || obj.get("ext_filter").is_some_and(has_non_empty_json_value))
    {
        obj.insert("files_only".to_string(), Value::Bool(true));
        obj.insert("dirs_only".to_string(), Value::Bool(false));
        obj.insert("names_only".to_string(), Value::Bool(true));
        obj.insert("include_hidden".to_string(), Value::Bool(include_hidden));
        info!("plan_contract_preserve_find_file_projection_for_directory_names");
        return;
    }
    let directory_filter_requested = structured_directory_filter_requested(obj);
    let file_filter_requested = structured_file_filter_requested(obj);
    obj.insert("files_only".to_string(), Value::Bool(false));
    obj.insert(
        "dirs_only".to_string(),
        Value::Bool(directory_filter_requested || file_filter_requested),
    );
    obj.insert("names_only".to_string(), Value::Bool(true));
    obj.insert("include_hidden".to_string(), Value::Bool(include_hidden));
    info!("plan_contract_enforce_directory_names_inventory");
}

pub(super) fn enforce_directory_entry_groups_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryEntryGroups) {
        return;
    }
    let Some(include_hidden) = route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden
    else {
        return;
    };
    if matches!(
        obj.get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default(),
        "list_dir" | "inventory_dir"
    ) {
        obj.insert("include_hidden".to_string(), Value::Bool(include_hidden));
        info!("plan_contract_enforce_directory_entry_groups_include_hidden");
    }
}

pub(super) fn enforce_file_names_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FileNames)
        || route.output_contract.delivery_intent == crate::OutputDeliveryIntent::DirectoryLookup
    {
        return;
    }
    let include_hidden = route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden
        .unwrap_or(false);
    let mut sort_by = obj
        .get("sort_by")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "name".to_string());
    if let Some(selector_sort_by) = route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .clone()
        .or_else(|| contract_hint_selector_sort_by(&route.route_reason))
    {
        sort_by = selector_sort_by;
        obj.insert("sort_by".to_string(), Value::String(sort_by.clone()));
    }
    let metadata_required = file_names_inventory_requires_metadata(route, &sort_by);
    obj.insert("files_only".to_string(), Value::Bool(true));
    obj.insert("dirs_only".to_string(), Value::Bool(false));
    obj.insert("names_only".to_string(), Value::Bool(!metadata_required));
    obj.insert("include_hidden".to_string(), Value::Bool(include_hidden));
    info!("plan_contract_enforce_file_names_inventory");
}

pub(super) fn requested_file_names_result_limit(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
) -> Option<u64> {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FileNames)
        || route.output_contract.delivery_required
    {
        return None;
    }
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .or_else(|| contract_hint_selector_limit(user_text))
        .or_else(|| original_user_text.and_then(contract_hint_selector_limit))
        .or_else(|| contract_hint_selector_limit(&route.route_reason))
}

pub(super) fn requested_file_names_inventory_sort_by(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
) -> String {
    route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .clone()
        .filter(|sort_by| {
            file_names_sort_by_has_structural_support(route, sort_by, user_text, original_user_text)
        })
        .or_else(|| contract_hint_selector_sort_by(user_text))
        .or_else(|| original_user_text.and_then(contract_hint_selector_sort_by))
        .or_else(|| contract_hint_selector_sort_by(&route.route_reason))
        .unwrap_or_else(|| "name".to_string())
}

pub(super) fn file_names_sort_by_has_structural_support(
    route: &RouteResult,
    sort_by: &str,
    user_text: &str,
    original_user_text: Option<&str>,
) -> bool {
    let sort_by = sort_by.trim();
    if !matches!(
        sort_by,
        "size_desc" | "size_asc" | "mtime_desc" | "mtime_asc"
    ) {
        return true;
    }
    if route
        .output_contract
        .self_extension
        .list_selector
        .include_metadata
        .is_some_and(|value| value)
    {
        return true;
    }
    if route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .as_deref()
        .is_some_and(|current_turn_sort_by| current_turn_sort_by == sort_by)
    {
        return true;
    }
    if file_names_route_requires_size_metadata(route) && matches!(sort_by, "size_desc" | "size_asc")
    {
        return true;
    }
    [
        Some(user_text),
        original_user_text,
        Some(route.route_reason.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter_map(contract_hint_selector_sort_by)
    .any(|hint| hint == sort_by)
}

pub(super) fn file_names_route_requires_size_metadata(route: &RouteResult) -> bool {
    route
        .output_contract
        .self_extension
        .list_selector
        .include_metadata
        .is_some_and(|value| value)
        || contract_hint_selector_include_metadata(&route.route_reason) == Some(true)
}

pub(super) fn file_names_inventory_requires_metadata(route: &RouteResult, sort_by: &str) -> bool {
    file_names_route_requires_size_metadata(route)
        || route
            .output_contract
            .self_extension
            .list_selector
            .include_metadata
            .is_some_and(|value| value)
        || matches!(
            sort_by.trim(),
            "size_desc" | "size_asc" | "mtime_desc" | "mtime_asc"
        )
}

pub(super) fn requested_file_paths_result_limit(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
) -> Option<u64> {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths)
        || route.output_contract.delivery_required
    {
        return None;
    }
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .or_else(|| contract_hint_selector_limit(user_text))
        .or_else(|| original_user_text.and_then(contract_hint_selector_limit))
        .or_else(|| contract_hint_selector_limit(&route.route_reason))
}

pub(super) fn enforce_file_paths_result_limit(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    action_name: &str,
    obj: &mut serde_json::Map<String, Value>,
) {
    let Some(limit) = requested_file_paths_result_limit(route, user_text, original_user_text)
    else {
        return;
    };
    let key = if action_name.eq_ignore_ascii_case("list_dir") {
        "max_entries"
    } else {
        "max_results"
    };
    if obj
        .get(key)
        .and_then(Value::as_u64)
        .is_some_and(|current| current == limit)
    {
        return;
    }
    obj.insert(key.to_string(), Value::Number(limit.into()));
    info!(
        "plan_contract_enforce_file_paths_result_limit action={} limit={}",
        action_name, limit
    );
}

pub(super) fn first_ext_filter_value(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let value = obj
        .get("ext_filter")
        .or_else(|| obj.get("ext"))
        .or_else(|| obj.get("extension"))
        .or_else(|| obj.get("extensions"))?;
    match value {
        Value::String(text) => normalize_extension_filter_text(text),
        Value::Array(items) => items
            .iter()
            .find_map(|item| item.as_str().and_then(normalize_extension_filter_text)),
        _ => None,
    }
}

pub(super) fn normalize_extension_filter_text(text: &str) -> Option<String> {
    text.trim()
        .trim_start_matches('.')
        .trim()
        .to_ascii_lowercase()
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn extension_from_globish_pattern(text: &str) -> Option<String> {
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

pub(super) fn should_rewrite_inventory_ext_filter_to_fs_basic(route: &RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths)
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::Path
        )
        && !route.output_contract.delivery_required
}

pub(super) fn rewrite_inventory_ext_filter_action_to_fs_basic(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    skill: &mut String,
    args: &mut Value,
) -> bool {
    if !should_rewrite_inventory_ext_filter_to_fs_basic(route) {
        return false;
    }
    let Some(obj) = args.as_object() else {
        return false;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(action_name, "inventory_dir" | "list_dir") {
        return false;
    }
    let Some(ext) = first_ext_filter_value(obj) else {
        return false;
    };
    let root = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let max_results = requested_file_paths_result_limit(route, user_text, original_user_text)
        .or_else(|| {
            obj.get("max_entries")
                .or_else(|| obj.get("limit"))
                .and_then(Value::as_u64)
        })
        .unwrap_or(100);
    *skill = "fs_basic".to_string();
    *args = serde_json::json!({
        "action": "find_entries",
        "root": root,
        "ext": ext,
        "target_kind": "file",
        "max_results": max_results,
        "recursive": true
    });
    info!("plan_contract_rewrite_inventory_ext_filter_to_fs_basic");
    true
}

pub(super) fn route_prefers_fs_search_name_result(route: &RouteResult) -> bool {
    !route.output_contract.delivery_required
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        && [
            crate::OutputSemanticKind::ScalarPathOnly,
            crate::OutputSemanticKind::FileNames,
            crate::OutputSemanticKind::DirectoryNames,
            crate::OutputSemanticKind::FilePaths,
        ]
        .iter()
        .any(|kind| route.output_contract_marker_is(*kind))
}

pub(super) fn enforce_fs_search_path_output_args(route: &RouteResult, args: &mut Value) -> bool {
    if !route_prefers_fs_search_name_result(route) {
        return false;
    }
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let is_grep_text = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action| action.eq_ignore_ascii_case("grep_text"));
    if obj
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        if let Some(path) = ["path", "dir", "directory", "search_root"]
            .iter()
            .find_map(|key| obj.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| Path::new(value).is_dir())
            .map(ToString::to_string)
        {
            obj.insert("root".to_string(), Value::String(path));
            changed = true;
        }
    }
    if !is_grep_text && !obj.contains_key("pattern") {
        if let Some(pattern) = ["basename_pattern", "name", "keyword", "query"]
            .iter()
            .find_map(|key| obj.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
        {
            obj.insert("pattern".to_string(), Value::String(pattern));
            changed = true;
        }
    }
    if !obj.contains_key("target_kind") {
        if obj
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|value| value.eq_ignore_ascii_case("file"))
        {
            obj.insert("target_kind".to_string(), Value::String("file".to_string()));
            changed = true;
        }
    }
    if !is_grep_text && route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths) {
        if let Some(ext) = first_ext_filter_value(obj).or_else(|| {
            obj.get("pattern")
                .and_then(Value::as_str)
                .and_then(extension_from_globish_pattern)
        }) {
            obj.insert("action".to_string(), Value::String("find_ext".to_string()));
            obj.insert("ext".to_string(), Value::String(ext));
            changed = true;
        }
    }
    let has_action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_action {
        if let Some(query) = obj
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
        {
            obj.insert("action".to_string(), Value::String("find_name".to_string()));
            obj.entry("pattern".to_string())
                .or_insert_with(|| Value::String(query));
            changed = true;
        }
    }
    changed
}

pub(super) fn action_is_workspace_summary_evidence(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } if skill == "list_dir" || skill == "read_file" => true,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "list_dir" | "read_text_range" | "stat_paths"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .is_some_and(|action| {
                matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "find_name"
                        | "find_path"
                        | "inventory_dir"
                        | "read_range"
                        | "tree_summary"
                        | "workspace_glance"
                )
            }),
        _ => false,
    }
}

pub(super) fn route_needs_unscoped_workspace_text_evidence(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route)
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract_is_unclassified()
}

pub(super) fn route_needs_workspace_synthesis_evidence(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route)
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && (route.output_contract_is_unclassified()
            || route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary))
}

pub(super) fn route_needs_workspace_summary_default_evidence(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route)
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary)
}

pub(super) fn route_needs_workspace_respond_only_default_evidence(route: &RouteResult) -> bool {
    route_needs_workspace_synthesis_evidence(route)
}

pub(super) fn route_disallows_unrequested_workspace_artifact_mutation(
    route: &RouteResult,
    loop_state: &LoopState,
) -> bool {
    route_needs_unscoped_workspace_text_evidence(route)
        && !route.wants_file_delivery
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && !loop_state.execution_recipe.is_active()
}

pub(super) fn route_locator_hint_is_path_like(route: &RouteResult) -> bool {
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    if matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return true;
    }
    let path = Path::new(hint);
    path.is_absolute()
        || path.components().count() > 1
        || path.extension().is_some()
        || hint.starts_with('.')
        || hint.starts_with('~')
}

pub(super) fn action_path_arg(args: &Value) -> Option<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
