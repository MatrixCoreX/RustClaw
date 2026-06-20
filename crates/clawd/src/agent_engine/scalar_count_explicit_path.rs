use super::*;

pub(super) fn scalar_count_explicit_count_path_from_actions(
    actions: &[AgentAction],
) -> Option<String> {
    let mut selected: Option<String> = None;
    for action in actions {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill.as_str(), args),
            _ => continue,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let is_count_action = (skill.eq_ignore_ascii_case("fs_basic")
            && action_name.eq_ignore_ascii_case("count_entries"))
            || (skill.eq_ignore_ascii_case("system_basic")
                && action_name.eq_ignore_ascii_case("count_inventory"));
        if !is_count_action {
            continue;
        }
        let Some(path) = args
            .get("path")
            .or_else(|| args.get("root"))
            .or_else(|| args.get("dir"))
            .or_else(|| args.get("directory"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        match &selected {
            None => selected = Some(path.to_string()),
            Some(existing) if existing == path => {}
            Some(_) => return None,
        }
    }
    selected
}

pub(super) fn apply_scalar_count_inventory_filters_from_actions(
    out: &mut serde_json::Map<String, Value>,
    actions: &[AgentAction],
) {
    for args in actions.iter().filter_map(action_structured_args) {
        let Some(obj) = args.as_object() else {
            continue;
        };
        for key in ["include_hidden", "recursive"] {
            if out.get(key).is_none() {
                if let Some(value) = obj.get(key).and_then(Value::as_bool) {
                    out.insert(key.to_string(), Value::Bool(value));
                }
            }
        }
        if out.get("ext_filter").is_none() {
            if let Some(value) = structured_ext_filter_arg(obj) {
                out.insert("ext_filter".to_string(), value);
            }
        }
    }
}

pub(super) fn structured_ext_filter_arg(obj: &serde_json::Map<String, Value>) -> Option<Value> {
    for key in ["ext_filter", "ext", "extension", "extensions"] {
        let Some(value) = obj.get(key) else {
            continue;
        };
        match value {
            Value::String(text) if !text.trim().is_empty() => {
                return Some(Value::String(text.trim().to_string()));
            }
            Value::Array(items)
                if items
                    .iter()
                    .any(|item| item.as_str().is_some_and(|text| !text.trim().is_empty())) =>
            {
                return Some(Value::Array(items.clone()));
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScalarCountInventoryKind {
    Any,
    Files,
    Dirs,
}

pub(super) fn scalar_count_inventory_kind_from_actions(
    actions: &[AgentAction],
) -> ScalarCountInventoryKind {
    let mut inferred: Option<ScalarCountInventoryKind> = None;
    for args in actions.iter().filter_map(action_structured_args) {
        let Some(kind) = scalar_count_inventory_kind_from_args(args) else {
            continue;
        };
        match inferred {
            None => inferred = Some(kind),
            Some(existing) if existing == kind => {}
            Some(_) => return ScalarCountInventoryKind::Any,
        }
    }
    inferred.unwrap_or(ScalarCountInventoryKind::Any)
}

pub(super) fn action_structured_args(action: &AgentAction) -> Option<&Value> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => Some(args),
        _ => None,
    }
}

pub(super) fn scalar_count_inventory_kind_from_args(
    args: &Value,
) -> Option<ScalarCountInventoryKind> {
    let obj = args.as_object()?;
    let files_only = structured_true_arg(
        args,
        &[
            "files_only",
            "file_only",
            "regular_files_only",
            "regular_file_only",
        ],
    );
    let dirs_only = structured_true_arg(
        args,
        &[
            "dirs_only",
            "dir_only",
            "directories_only",
            "directory_only",
            "folders_only",
            "folder_only",
        ],
    );
    if files_only && !dirs_only {
        return Some(ScalarCountInventoryKind::Files);
    }
    if dirs_only && !files_only {
        return Some(ScalarCountInventoryKind::Dirs);
    }
    if structured_ext_filter_arg(obj).is_some() {
        return Some(ScalarCountInventoryKind::Files);
    }

    let count_files = structured_bool_arg(args, "count_files");
    let count_dirs = structured_bool_arg(args, "count_dirs");
    match (count_files, count_dirs) {
        (Some(true), Some(false)) => return Some(ScalarCountInventoryKind::Files),
        (Some(false), Some(true)) => return Some(ScalarCountInventoryKind::Dirs),
        _ => {}
    }

    for key in [
        "kind_filter",
        "filter_kind",
        "target_kind",
        "kind",
        "entry_kind",
        "entry_type",
        "item_kind",
        "item_type",
    ] {
        if let Some(kind) = obj
            .get(key)
            .and_then(Value::as_str)
            .and_then(scalar_count_inventory_kind_from_structured_value)
        {
            return Some(kind);
        }
    }
    None
}

pub(super) fn structured_true_arg(args: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| matches!(structured_bool_arg(args, key), Some(true)))
}

pub(super) fn structured_bool_arg(args: &Value, key: &str) -> Option<bool> {
    match args.as_object()?.get(key)? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn scalar_count_inventory_kind_from_structured_value(
    value: &str,
) -> Option<ScalarCountInventoryKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "file" | "files" | "regular_file" | "regular_files" => {
            Some(ScalarCountInventoryKind::Files)
        }
        "dir" | "dirs" | "directory" | "directories" | "folder" | "folders" => {
            Some(ScalarCountInventoryKind::Dirs)
        }
        _ => None,
    }
}

pub(super) fn hidden_entries_count_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        || !route_requests_hidden_entries_count(route)
    {
        return None;
    }
    route_directory_locator_path(route, auto_locator_path)
}

pub(super) fn route_directory_locator_path(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    let current_workspace_fallback =
        route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace;
    let hint_looks_like_path = hint.contains(['/', '\\']) || hint.starts_with('.');
    let auto_dir = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).is_dir());
    if !hint.is_empty() {
        if let Some(path) = auto_dir.filter(|path| locator_path_matches_hint(path, hint)) {
            return Some(path.to_string());
        }
        if Path::new(hint).is_dir()
            || matches!(
                route.output_contract.locator_kind,
                crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
            )
            || hint_looks_like_path
        {
            return Some(hint.to_string());
        }
    }
    auto_dir
        .or_else(|| (current_workspace_fallback || hint.is_empty()).then_some("."))
        .map(ToString::to_string)
}

pub(super) fn locator_path_matches_hint(path: &str, hint: &str) -> bool {
    let path = path.trim().trim_end_matches(['/', '\\']);
    let hint = hint.trim().trim_end_matches(['/', '\\']);
    if path.is_empty() || hint.is_empty() {
        return false;
    }
    if path.eq_ignore_ascii_case(hint) {
        return true;
    }
    path.ends_with(&format!("/{hint}")) || path.ends_with(&format!("\\{hint}"))
}

pub(super) fn route_requests_hidden_entries_count(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck
}

pub(super) fn replace_hidden_entries_count_plan_with_inventory_dir(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    let Some(path) = hidden_entries_count_locator_path(route_result, auto_locator_path) else {
        return actions;
    };
    info!("plan_replace_hidden_entries_count_plan_with_inventory_dir");
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": path,
            "include_hidden": true,
            "names_only": true,
            "max_entries": 1000,
        }),
    }]
}

pub(super) fn route_requests_service_status(route: &RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        || (route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
            && route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && route.output_contract.locator_kind == crate::OutputLocatorKind::None)
}

pub(super) fn safe_service_status_target(raw: &str) -> Option<String> {
    let target = raw.trim().trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
    if target.is_empty()
        || !target
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return None;
    }
    Some(target.to_string())
}

pub(super) fn shell_like_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

pub(super) fn command_basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

pub(super) fn token_starts_new_shell_clause(token: &str) -> bool {
    let trimmed = token.trim();
    trimmed.is_empty()
        || matches!(
            trimmed,
            ">" | ">>" | "2>" | "2>>" | "<" | "|" | "||" | "&&" | ";"
        )
        || trimmed.contains('|')
        || trimmed.contains(';')
        || trimmed.contains('<')
        || trimmed.contains('>')
        || trimmed == "if"
        || trimmed == "then"
        || trimmed == "else"
        || trimmed == "fi"
}

pub(super) fn pgrep_status_target(words: &[String]) -> Option<String> {
    if words
        .first()
        .map(|word| command_basename(word).eq_ignore_ascii_case("pgrep"))
        != Some(true)
    {
        return None;
    }
    words
        .iter()
        .skip(1)
        .take_while(|word| !token_starts_new_shell_clause(word))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find(|word| !word.starts_with('-'))
        .and_then(|word| safe_service_status_target(word))
}

pub(super) fn systemctl_status_target(words: &[String]) -> Option<(String, Option<&'static str>)> {
    if words
        .first()
        .map(|word| command_basename(word).eq_ignore_ascii_case("systemctl"))
        != Some(true)
    {
        return None;
    }
    let subcommand_idx = words.iter().position(|word| {
        let word = word.trim();
        word.eq_ignore_ascii_case("is-active") || word.eq_ignore_ascii_case("status")
    })?;
    words
        .iter()
        .skip(subcommand_idx + 1)
        .find(|word| !word.starts_with('-'))
        .and_then(|word| safe_service_status_target(word))
        .map(|target| (target, Some("systemd")))
}

pub(super) fn service_command_status_target(
    words: &[String],
) -> Option<(String, Option<&'static str>)> {
    if words
        .first()
        .map(|word| command_basename(word).eq_ignore_ascii_case("service"))
        != Some(true)
        || words.len() < 3
        || !words[2].eq_ignore_ascii_case("status")
    {
        return None;
    }
    safe_service_status_target(&words[1]).map(|target| (target, Some("service")))
}

pub(super) fn service_status_command_target(
    command: &str,
) -> Option<(String, Option<&'static str>)> {
    let words = shell_like_words(command);
    pgrep_status_target(&words)
        .map(|target| (target, None))
        .or_else(|| systemctl_status_target(&words))
        .or_else(|| service_command_status_target(&words))
}

pub(super) fn service_status_target_for_action(
    route: &RouteResult,
    action: &AgentAction,
) -> Option<(String, Option<&'static str>)> {
    let route_target = safe_service_status_target(route.output_contract.locator_hint.trim());
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            let command = if skill == "run_cmd" {
                args.get("command").and_then(Value::as_str)
            } else if skill == "system_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.trim().eq_ignore_ascii_case("run_cmd"))
            {
                args.get("command").and_then(Value::as_str)
            } else {
                None
            };
            command
                .and_then(service_status_command_target)
                .or_else(|| route_target.map(|target| (target, None)))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => None,
    }
}

pub(super) fn rewrite_service_status_plan_to_service_control(
    route_result: Option<&RouteResult>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requests_service_status(route) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let should_consider = matches!(
            action,
            AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
                if {
                    let skill = skill.trim().to_ascii_lowercase();
                    skill == "run_cmd" || skill == "system_basic"
                }
        );
        if !should_consider {
            continue;
        }
        let Some((target, manager_type)) = service_status_target_for_action(route, action) else {
            continue;
        };
        let mut args = serde_json::json!({
            "action": "status",
            "target": target,
        });
        if let Some(manager_type) = manager_type {
            args["manager_type"] = Value::String(manager_type.to_string());
        }
        *action = AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args,
        };
        changed = true;
    }
    if changed {
        while rewritten.last().is_some_and(is_discussion_followup_action) {
            rewritten.pop();
        }
        info!("plan_rewrite_service_status_to_service_control");
    }
    rewritten
}

pub(super) fn is_service_control_status_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.trim().eq_ignore_ascii_case("service_control")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.trim().eq_ignore_ascii_case("status"))
    )
}

pub(super) fn strip_service_status_discussion_actions(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ServiceStatus
        || !actions.iter().any(is_service_control_status_action)
        || !actions.iter().any(is_discussion_followup_action)
    {
        return actions;
    }
    let rewritten = actions
        .into_iter()
        .filter(|action| !is_discussion_followup_action(action))
        .collect::<Vec<_>>();
    if rewritten.is_empty() {
        return rewritten;
    }
    info!("plan_strip_service_status_discussion_actions");
    rewritten
}

pub(super) fn structured_keys_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::StructuredKeys
    {
        return None;
    }
    let hint = route.output_contract.locator_hint.trim();
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| (!hint.is_empty()).then_some(hint))
        .filter(|path| Path::new(path).is_file())
        .map(ToString::to_string)
}

pub(super) fn replace_structured_keys_read_plan(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                if (skill.eq_ignore_ascii_case("config_basic")
                    && args.get("action").and_then(Value::as_str) == Some("list_keys")
                    && args
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                        .is_some())
                    || (skill.eq_ignore_ascii_case("system_basic")
                        && args.get("action").and_then(Value::as_str) == Some("structured_keys"))
        )
    }) {
        return actions;
    }
    let strict_structured_keys_contract = route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::StructuredKeys
            && route.output_contract.response_shape == crate::OutputResponseShape::Strict
    });
    if !strict_structured_keys_contract
        && actions
            .iter()
            .any(action_is_structured_field_read_with_explicit_field)
    {
        return actions;
    }
    let Some(path) = structured_keys_locator_path(route_result, auto_locator_path) else {
        return actions;
    };
    if !actions.iter().any(|action| {
        matches!(
            action,
                AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args }
                    if (skill.eq_ignore_ascii_case("fs_basic")
                        && args.get("action").and_then(Value::as_str) == Some("read_text_range"))
                        || (skill.eq_ignore_ascii_case("config_basic")
                            && matches!(
                                args.get("action").and_then(Value::as_str),
                                Some("read_field" | "read_fields" | "validate")
                            ))
                        || (skill.eq_ignore_ascii_case("system_basic")
                            && matches!(
                                args.get("action").and_then(Value::as_str),
                                Some("read_range" | "extract_field" | "extract_fields" | "validate_structured")
                            ))
        )
    }) {
        return actions;
    }
    info!("plan_replace_structured_keys_read_plan_with_structured_keys");
    vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "list_keys",
            "path": path,
            "max_keys": 1000,
        }),
    }]
}

pub(super) fn has_structured_keys_observation(loop_state: &LoopState, path: &str) -> bool {
    let requested_path = path.trim();
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "config_basic") {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output) else {
            return false;
        };
        if value.get("action").and_then(Value::as_str) != Some("structured_keys") {
            return false;
        }
        if !value
            .get("exists")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return false;
        }
        if !value.get("keys").is_some_and(Value::is_array)
            && !value.get("identity_values").is_some_and(Value::is_array)
            && !value.get("indices_preview").is_some_and(Value::is_array)
        {
            return false;
        }
        if requested_path.is_empty() {
            return true;
        }
        value
            .get("resolved_path")
            .or_else(|| value.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|observed| observed == requested_path)
    })
}

pub(super) fn structured_keys_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    user_text: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    let path = structured_keys_locator_path(route_result, auto_locator_path)?;
    if has_structured_keys_observation(loop_state, &path) {
        return None;
    }
    let enabled_skills = state.get_skills_list();
    if !enabled_skills.is_empty() && !enabled_skills.contains("config_basic") {
        return None;
    }
    let field_path = structured_current_turn_field_selectors(route, user_text, true, Some(&path))
        .into_iter()
        .next();
    if let Some(field_path) = field_path.as_deref() {
        if structured_field_path_resolves_scalar_value(&path, field_path) {
            let actions = vec![config_basic_read_field_action(
                path.to_string(),
                field_path.to_string(),
            )];
            let raw_plan_text = serde_json::json!({
                "steps": [{
                    "type": "call_tool",
                    "tool": "config_basic",
                    "args": actions
                        .first()
                        .and_then(|action| match action {
                            AgentAction::CallTool { args, .. } => Some(args.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| serde_json::json!({})),
                }]
            })
            .to_string();
            return Some(build_plan_result(
                goal,
                &raw_plan_text,
                if loop_state.round_no <= 1 {
                    PlanKind::Single
                } else {
                    PlanKind::Incremental
                },
                &actions,
            ));
        }
    }
    let mut args = serde_json::json!({
        "action": "list_keys",
        "path": path,
        "max_keys": 1000,
    });
    if let Some(field_path) = field_path {
        args["field_path"] = Value::String(field_path);
    }
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args,
    }];
    let raw_plan_text = serde_json::json!({
        "steps": [{
            "type": "call_tool",
            "tool": "config_basic",
            "args": actions
                .first()
                .and_then(|action| match action {
                    AgentAction::CallTool { args, .. } => Some(args.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| serde_json::json!({})),
        }]
    })
    .to_string();
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

pub(super) fn action_is_structured_field_read_with_explicit_field(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_ascii_lowercase());
            let is_field_read = if skill.eq_ignore_ascii_case("config_basic") {
                matches!(action_name.as_deref(), Some("read_field" | "read_fields"))
            } else if skill.eq_ignore_ascii_case("system_basic") {
                matches!(
                    action_name.as_deref(),
                    Some("extract_field" | "extract_fields")
                )
            } else {
                false
            };
            if !is_field_read {
                return false;
            }
            if json_trimmed_string_arg(args, &["field_path", "field", "key"]).is_some() {
                return true;
            }
            let field_count = args
                .get("field_paths")
                .or_else(|| args.get("fields"))
                .map(|value| string_list_from_value(Some(value)).len())
                .unwrap_or_default();
            field_count == 1
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Think { .. } => false,
    }
}

pub(super) fn action_observes_bounded_file_content(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim();
            if skill.eq_ignore_ascii_case("run_cmd") {
                return run_cmd_command_from_args(args)
                    .and_then(readonly_file_read_from_shell_command)
                    .is_some();
            }
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            ((skill.eq_ignore_ascii_case("system_basic")
                && action.eq_ignore_ascii_case("read_range"))
                || (skill.eq_ignore_ascii_case("fs_basic")
                    && action.eq_ignore_ascii_case("read_text_range")))
                || skill.eq_ignore_ascii_case("read_file")
                || (skill.eq_ignore_ascii_case("doc_parse")
                    && action.eq_ignore_ascii_case("parse_doc"))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => false,
    }
}

pub(super) fn planned_bounded_file_read_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim();
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let is_bounded_read = (skill.eq_ignore_ascii_case("system_basic")
                && action.eq_ignore_ascii_case("read_range"))
                || (skill.eq_ignore_ascii_case("fs_basic")
                    && action.eq_ignore_ascii_case("read_text_range"))
                || skill.eq_ignore_ascii_case("read_file");
            is_bounded_read
                .then(|| args.get("path").and_then(Value::as_str))
                .flatten()
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => None,
    }
}

fn planned_bounded_file_read_requests_raw_slice(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.trim(), args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    let Some(obj) = args.as_object() else {
        return false;
    };
    if !is_read_range_action(skill, obj) {
        return false;
    }
    if obj.get("start_line").is_some()
        || obj.get("end_line").is_some()
        || obj.get("line_start").is_some()
        || obj.get("line_end").is_some()
    {
        return true;
    }
    obj.get("mode")
        .or_else(|| obj.get("range"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
        .is_some_and(|mode| {
            !mode.eq_ignore_ascii_case("head")
                && !mode.eq_ignore_ascii_case("full")
                && !mode.eq_ignore_ascii_case("all")
        })
}

pub(super) fn planned_structured_config_observation_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. }
            if action_is_readonly_config_observation(action) =>
        {
            args.get("path").and_then(Value::as_str).map(str::trim)
        }
        _ => None,
    }
    .filter(|path| !path.is_empty())
}

pub(super) fn route_allows_single_document_parse_synthesis(route: &RouteResult) -> bool {
    !route.needs_clarify
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
}

pub(super) fn route_contract_allows_doc_parse(route: &RouteResult) -> bool {
    crate::contract_matrix::allowed_action_refs_for_output_contract(&route.output_contract)
        .iter()
        .any(|action_ref| {
            action_ref.skill == "doc_parse"
                && action_ref
                    .action
                    .as_deref()
                    .is_none_or(|action| action == "parse_doc")
        })
}

pub(super) fn prefer_doc_parse_for_single_document_synthesis(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output
        || !doc_parse_is_enabled(state)
        || !actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    ) {
        return actions;
    }
    if !route_allows_single_document_parse_synthesis(route) {
        return actions;
    }
    if !route_contract_allows_doc_parse(route) {
        return actions;
    }
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut changed = false;
    for action in actions {
        if let Some(path) = planned_bounded_file_read_path(&action)
            .filter(|path| Path::new(path).is_file())
            .filter(|path| doc_parse_supported_path(path))
            .filter(|path| !repo_text_artifact_prefers_bounded_fs_read(path))
        {
            rewritten.push(AgentAction::CallSkill {
                skill: "doc_parse".to_string(),
                args: serde_json::json!({
                    "action": "parse_doc",
                    "path": path,
                    "mode": "auto",
                    "max_chars": 12000,
                    "include_metadata": true
                }),
            });
            changed = true;
        } else {
            rewritten.push(action);
        }
    }
    if changed {
        info!("plan_prefer_doc_parse_for_single_document_synthesis");
    }
    rewritten
}

pub(super) fn prefer_log_analyze_for_single_log_synthesis(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output
        || !log_analyze_is_enabled(state)
        || !actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    if route
        .output_contract
        .semantic_kind
        .is_content_excerpt_summary()
    {
        return actions;
    }
    if !route_allows_single_document_parse_synthesis(route) {
        return actions;
    }
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut changed = false;
    for action in actions {
        if planned_bounded_file_read_requests_raw_slice(&action) {
            rewritten.push(action);
            continue;
        }
        if let Some(path) = planned_bounded_file_read_path(&action)
            .filter(|path| Path::new(path).is_file())
            .filter(|path| log_analyze_supported_path(path))
            .filter(|path| contract_allows_log_analyze_for_path(route, path))
        {
            rewritten.push(AgentAction::CallSkill {
                skill: "log_analyze".to_string(),
                args: serde_json::json!({
                    "path": path,
                    "max_matches": 50
                }),
            });
            changed = true;
        } else {
            rewritten.push(action);
        }
    }
    if changed {
        info!("plan_prefer_log_analyze_for_single_log_synthesis");
    }
    rewritten
}

pub(super) fn existence_path_summary_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::ExistenceWithPathSummary
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| !Path::new(path).is_dir())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty() && Path::new(hint).is_file()).then_some(hint)
        })
        .map(ToString::to_string)
}

pub(super) fn ensure_existence_path_summary_has_bounded_content(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    let should_handle = route_result.is_some_and(|route| {
        !route.needs_clarify
            && !route.output_contract.delivery_required
            && route.output_contract.semantic_kind
                == crate::OutputSemanticKind::ExistenceWithPathSummary
    });
    if !should_handle {
        return actions;
    };
    let target_path = existence_path_summary_target_path(route_result, auto_locator_path);
    let mut rewritten = actions;
    if let Some(path) = target_path.filter(|path| !Path::new(path).is_dir()) {
        if !rewritten.iter().any(action_observes_bounded_file_content)
            && !path_metadata_facts_response_is_sufficient(&rewritten)
        {
            let insert_at = rewritten
                .iter()
                .position(|action| {
                    matches!(
                        action,
                        AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
                    )
                })
                .unwrap_or(rewritten.len());
            rewritten.insert(
                insert_at,
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "read_text_range",
                        "path": path,
                        "mode": "head",
                        "n": 30
                    }),
                },
            );
            info!("plan_insert_existence_path_summary_read_range");
        }
    }
    if !rewritten
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        let evidence_refs = observation_action_evidence_refs(&rewritten);
        if !evidence_refs.is_empty() {
            let insert_at = rewritten
                .iter()
                .rposition(|action| matches!(action, AgentAction::Respond { .. }))
                .unwrap_or(rewritten.len());
            rewritten.insert(
                insert_at,
                AgentAction::SynthesizeAnswer {
                    evidence_refs: evidence_refs.clone(),
                },
            );
            match rewritten.get_mut(insert_at + 1) {
                Some(AgentAction::Respond { content }) => {
                    *content = "{{last_output}}".to_string();
                }
                _ => rewritten.push(AgentAction::Respond {
                    content: "{{last_output}}".to_string(),
                }),
            }
            info!(
                "plan_insert_existence_path_summary_synthesis refs={}",
                evidence_refs.join(",")
            );
        }
    }
    rewritten
}

pub(super) fn planned_action_is_path_metadata_facts(action: &AgentAction) -> bool {
    let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
        return false;
    };
    let Some(action_name) = action_args(action)
        .and_then(|args| args.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
    else {
        return false;
    };
    matches!(
        (
            skill.to_ascii_lowercase().as_str(),
            action_name.to_ascii_lowercase().as_str()
        ),
        ("fs_basic", "stat_paths")
            | ("fs_basic", "compare_paths")
            | ("system_basic", "path_batch_facts")
            | ("system_basic", "compare_paths")
    )
}

pub(super) fn path_metadata_facts_action_requests_metadata_fields(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. }) = action else {
        return false;
    };
    let Some(fields) = args.get("fields").and_then(Value::as_array) else {
        return false;
    };
    fields.iter().any(|field| {
        field.as_str().map(str::trim).is_some_and(|field| {
            matches!(
                field.to_ascii_lowercase().as_str(),
                "size" | "size_bytes" | "exists" | "kind" | "modified" | "modified_ts"
            )
        })
    })
}

pub(super) fn path_metadata_facts_response_is_sufficient(actions: &[AgentAction]) -> bool {
    if !actions.iter().any(planned_action_is_path_metadata_facts) {
        return false;
    }
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return false;
    };
    if !content.contains("{{") {
        return false;
    }
    let metadata_fields = [
        "size",
        "size_bytes",
        "exists",
        "kind",
        "modified",
        "modified_ts",
    ];
    let content_lower = content.to_ascii_lowercase();
    let placeholder_refs = extract_output_placeholder_evidence_refs(content);
    let response_mentions_metadata = metadata_fields
        .iter()
        .any(|field| content_lower.contains(field));
    let placeholder_mentions_metadata = placeholder_refs.iter().any(|reference| {
        let reference = reference.trim().to_ascii_lowercase();
        metadata_fields
            .iter()
            .any(|field| reference == *field || reference.ends_with(&format!(".{field}")))
    });
    placeholder_mentions_metadata
        || response_mentions_metadata
        || actions
            .iter()
            .filter(|action| planned_action_is_path_metadata_facts(action))
            .any(path_metadata_facts_action_requests_metadata_fields)
}
