use super::*;
use std::collections::HashMap;

pub(super) fn is_read_range_action(skill: &str, obj: &serde_json::Map<String, Value>) -> bool {
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    (skill.eq_ignore_ascii_case("fs_basic") && action.eq_ignore_ascii_case("read_text_range"))
        || (skill.eq_ignore_ascii_case("system_basic") && action.eq_ignore_ascii_case("read_range"))
}

pub(super) fn read_range_has_explicit_bounds(obj: &serde_json::Map<String, Value>) -> bool {
    if obj.get("n").is_some()
        || obj.get("start_line").is_some()
        || obj.get("end_line").is_some()
        || obj.get("line_start").is_some()
        || obj.get("line_end").is_some()
    {
        return true;
    }
    obj.get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
        .is_some_and(|mode| {
            !mode.eq_ignore_ascii_case("head")
                && !mode.eq_ignore_ascii_case("full")
                && !mode.eq_ignore_ascii_case("all")
        })
}

pub(super) fn path_has_structured_text_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| matches!(ext.as_str(), "json" | "toml" | "yaml" | "yml"))
}

pub(super) fn structured_config_format_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Some("json"),
        Some("toml") => Some("toml"),
        Some("yaml" | "yml") => Some("yaml"),
        _ => None,
    }
}

pub(super) fn config_basic_validate_action(path: String) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert("action".to_string(), Value::String("validate".to_string()));
    args.insert("path".to_string(), Value::String(path.clone()));
    if let Some(format) = structured_config_format_for_path(&path) {
        args.insert("format".to_string(), Value::String(format.to_string()));
    }
    args.insert(
        "validation_profile".to_string(),
        Value::String("syntax_only".to_string()),
    );
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: Value::Object(args),
    }
}

pub(super) fn action_is_structured_config_validation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            skill.eq_ignore_ascii_case("config_basic")
                && action_name.eq_ignore_ascii_case("validate")
        }
        _ => false,
    }
}

pub(super) fn config_validation_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    actions
        .iter()
        .find_map(planned_bounded_file_read_path)
        .or_else(|| {
            actions
                .iter()
                .find_map(planned_structured_config_observation_path)
        })
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| !path.is_empty())
        })
        .filter(|path| path_has_structured_document_extension(path))
        .map(ToString::to_string)
}

pub(super) fn rewrite_config_validation_read_plan_to_validate(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || !route_has_config_validation_contract(route)
        || actions.iter().any(action_is_structured_config_validation)
    {
        return actions;
    }
    let Some(path) = config_validation_target_path(Some(route), auto_locator_path, &actions) else {
        return actions;
    };

    let mut rewritten = Vec::with_capacity(actions.len().max(1));
    let mut inserted = false;
    let mut changed = false;
    for action in actions {
        if !inserted
            && (action_observes_bounded_file_content(&action)
                || action_is_readonly_config_observation(&action))
        {
            rewritten.push(config_basic_validate_action(path.clone()));
            inserted = true;
            changed = true;
            continue;
        }
        if !inserted
            && matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        {
            rewritten.push(config_basic_validate_action(path.clone()));
            inserted = true;
            changed = true;
        }
        rewritten.push(action);
    }
    if !inserted {
        rewritten.push(config_basic_validate_action(path.clone()));
        changed = true;
    }
    if changed {
        info!(
            "plan_rewrite_config_validation_read_plan_to_validate path={}",
            crate::truncate_for_log(&path)
        );
    }
    rewritten
}

pub(super) fn rewrite_unrequested_path_like_config_field_read_to_validate(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
    {
        return actions;
    }

    let mut changed = false;
    let mut rewritten = Vec::with_capacity(actions.len());
    for action in actions {
        let Some(path) = unrequested_path_like_config_field_validation_path(
            state,
            route,
            user_text,
            auto_locator_path,
            &action,
        ) else {
            rewritten.push(action);
            continue;
        };
        info!(
            "plan_rewrite_unrequested_path_like_config_field_read_to_validate path={}",
            crate::truncate_for_log(&path)
        );
        rewritten.push(config_basic_validate_action(path));
        changed = true;
    }

    if changed {
        rewritten
    } else {
        rewritten
    }
}

pub(super) fn unrequested_path_like_config_field_validation_path(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
    auto_locator_path: Option<&str>,
    action: &AgentAction,
) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return None,
    };
    if !skill.eq_ignore_ascii_case("system_basic") && !skill.eq_ignore_ascii_case("config_basic") {
        return None;
    }

    let request = structured_extract_request(args)?;
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, &request.path);
    let current_text = current.display().to_string();
    if !path_has_structured_text_extension(&request.path)
        && !path_has_structured_text_extension(&current_text)
    {
        return None;
    }
    if structured_file_has_all_fields(&current, &request.fields) {
        return None;
    }
    if request
        .fields
        .iter()
        .any(|field| current_request_mentions_token(user_text, route, field))
    {
        return None;
    }
    if structured_scalar_field_selector(route, user_text, true, None, Some(&current_text)).is_some()
    {
        return None;
    }
    if !request
        .fields
        .iter()
        .any(|field| field_token_looks_like_locator(field))
    {
        return None;
    }

    let auto_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| path_has_structured_text_extension(path))
        .filter(|path| {
            same_existing_or_display_path(Path::new(path), &current) || !current.exists()
        });
    auto_path
        .map(ToString::to_string)
        .or_else(|| path_has_structured_text_extension(&request.path).then(|| request.path.clone()))
        .or_else(|| path_has_structured_text_extension(&current_text).then_some(current_text))
}

pub(super) fn field_token_looks_like_locator(value: &str) -> bool {
    let token = value.trim();
    if token.is_empty() {
        return false;
    }
    Path::new(token).components().count() > 1 || filename_candidate_has_document_extension(token)
}

pub(super) fn current_request_mentions_token(
    user_text: &str,
    route: &RouteResult,
    token: &str,
) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    let token_lower = token.to_ascii_lowercase();
    [user_text, route.resolved_intent.as_str()]
        .iter()
        .any(|text| text.contains(token) || text.to_ascii_lowercase().contains(&token_lower))
}

pub(super) fn normalize_evidence_contract_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = rewrite_active_anchor_basename_file_reads_to_bound_target(plan_context, actions);
    let actions = replace_file_paths_anchor_respond_only_with_find_entries(
        route_result,
        plan_context,
        actions,
    );
    let actions = replace_scalar_path_anchor_respond_only_with_stat_paths(
        route_result,
        plan_context,
        actions,
    );
    let actions = replace_content_evidence_synthesize_only_with_file_reads(
        state,
        route_result,
        loop_state,
        user_text,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = ensure_explicit_multi_file_targets_have_content_reads(
        state,
        route_result,
        loop_state,
        user_text,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = replace_workspace_synthesis_respond_only_plan(route_result, loop_state, actions);
    let actions = rewrite_extract_field_alias_args(actions);
    let actions = rewrite_action_ref_tool_calls(actions);
    let actions = rewrite_config_change_preview_to_config_edit_plan(
        route_result,
        user_text,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_config_mutation_plan_only_to_config_edit_plan(
        route_result,
        loop_state,
        user_text,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_config_mutation_to_config_edit_closed_loop(
        route_result,
        loop_state,
        user_text,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_structured_multi_field_read_plan_to_read_fields(
        route_result,
        user_text,
        true,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_structured_scalar_field_read_plan_to_read_field(
        state,
        route_result,
        user_text,
        true,
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_scalar_candidate_respond_to_structured_field_read(
        state,
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = add_prior_structured_text_field_read_for_scalar_compare(
        state,
        route_result,
        loop_state,
        user_text,
        plan_context,
        actions,
    );
    let actions =
        rewrite_config_validation_read_plan_to_validate(route_result, auto_locator_path, actions);
    let actions = rewrite_unrequested_path_like_config_field_read_to_validate(
        state,
        route_result,
        user_text,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_extract_field_paths_to_structured_candidates(
        state,
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        canonicalize_quantity_compare_structured_field_reads(state, route_result, actions);
    let actions = prune_unscoped_workspace_summary_evidence_for_scope(state, route_result, actions);
    let actions =
        strip_unrequested_workspace_artifact_mutations(state, route_result, loop_state, actions);
    let actions =
        ensure_workspace_synthesis_has_default_text_evidence(route_result, loop_state, actions);
    let actions =
        append_synthesize_for_unscoped_workspace_text_evidence(route_result, loop_state, actions);
    let actions = rewrite_dir_compare_paths_to_unique_workspace_directories(state, actions);
    let actions = replace_directory_compare_search_plan(state, route_result, actions);
    let actions = rewrite_split_dir_basename_stat_paths_to_auto_locator_file(
        state,
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_constructed_missing_stat_path_to_exact_find_entries(
        state,
        route_result,
        user_text,
        actions,
    );
    let actions =
        canonicalize_quantity_compare_structured_field_reads(state, route_result, actions);
    let actions = ensure_explicit_multi_file_targets_have_path_facts(
        route_result,
        loop_state,
        user_text,
        actions,
    );
    let actions = ensure_existence_multi_file_targets_have_path_facts(
        route_result,
        loop_state,
        user_text,
        actions,
    );
    let actions = append_synthesize_answer_for_structured_scalar_compare(route_result, actions);
    let actions =
        rewrite_unresolved_template_arg_multi_file_read_plan(route_result, user_text, actions);
    let actions = strip_unresolved_template_reads_after_inventory_dir(actions);
    let actions =
        strip_workspace_synthesis_without_text_evidence(route_result, loop_state, actions);
    actions
}

pub(super) fn rewrite_action_ref_tool_calls(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallTool { tool, args } => {
                let (tool, args, changed) = rewrite_action_ref_call(tool, args);
                if changed {
                    info!(
                        "plan_rewrite_action_ref_tool_call tool={}",
                        crate::truncate_for_log(&tool)
                    );
                }
                AgentAction::CallTool { tool, args }
            }
            AgentAction::CallSkill { skill, args } => {
                let (skill, args, changed) = rewrite_action_ref_call(skill, args);
                if changed {
                    info!(
                        "plan_rewrite_action_ref_skill_call skill={}",
                        crate::truncate_for_log(&skill)
                    );
                }
                AgentAction::CallSkill { skill, args }
            }
            other => other,
        })
        .collect()
}

fn rewrite_action_ref_call(raw_skill: String, args: Value) -> (String, Value, bool) {
    let mut skill = raw_skill;
    let mut args = args;
    let mut changed = false;
    if let Some((skill_part, action_part)) =
        skill.split_once('.').map(|(skill_part, action_part)| {
            (
                skill_part.trim().to_string(),
                action_part.trim().to_string(),
            )
        })
    {
        if !skill_part.is_empty() && !action_part.is_empty() {
            skill = skill_part;
            if let Some(obj) = args.as_object_mut() {
                obj.entry("action".to_string())
                    .or_insert_with(|| Value::String(action_part));
            }
            changed = true;
        }
    }
    if normalize_config_edit_value_aliases(&skill, &mut args) {
        changed = true;
    }
    (skill, args, changed)
}

fn normalize_config_edit_value_aliases(skill: &str, args: &mut Value) -> bool {
    if !skill.eq_ignore_ascii_case("config_edit") {
        return false;
    }
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if obj.contains_key("value") {
        return false;
    }
    let Some(value) = ["new_value", "target_value"]
        .into_iter()
        .find_map(|alias| obj.remove(alias))
    else {
        return false;
    };
    obj.insert("value".to_string(), value);
    true
}

fn route_has_config_change_contract(route: &RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["config"],
        &[
            "apply_change",
            "apply_config_change",
            "plan_change",
            "plan_config_change",
            "set_field",
            "write_field",
        ],
    )
}

fn route_has_config_validation_contract(route: &RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::ConfigValidation)
        || crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["config"],
            &["validate"],
        )
}

fn route_has_config_risk_contract(route: &RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::ConfigRiskAssessment)
        || crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["config"],
            &["guard_after_change", "guard_config"],
        )
}

pub(super) fn rewrite_config_mutation_plan_only_to_config_edit_plan(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route_has_config_change_contract(route)
        || loop_state.execution_recipe.kind
            == crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
        || actions.iter().any(action_is_obvious_mutation)
    {
        return actions;
    }
    let Some(parsed) = parse_config_change_preview(user_text, route, auto_locator_path) else {
        return actions;
    };
    info!(
        "plan_rewrite_config_mutation_plan_only_to_config_edit_plan path={} field={}",
        crate::truncate_for_log(&parsed.path),
        crate::truncate_for_log(&parsed.field_path)
    );
    vec![config_edit_change_action("plan_config_change", &parsed)]
}

pub(super) fn rewrite_active_anchor_basename_file_reads_to_bound_target(
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let anchor = active_anchor_plan_context(plan_context);
    let Some(bound_target) = anchor
        .bound_target
        .as_deref()
        .map(str::trim)
        .filter(|target| !target.is_empty())
    else {
        return actions;
    };
    if anchor.ordered_entries.is_empty() {
        return actions;
    }
    let bound_path = Path::new(bound_target);
    if !bound_path.is_dir() {
        return actions;
    }

    let mut rewrites = HashMap::<String, String>::new();
    for entry in &anchor.ordered_entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let entry_path = Path::new(entry);
        let target = if entry_path.is_absolute() || entry_path.components().count() > 1 {
            entry_path.to_path_buf()
        } else {
            bound_path.join(entry_path)
        };
        if !target.is_file() {
            continue;
        }
        let Some(basename) = target
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        rewrites
            .entry(basename.to_ascii_lowercase())
            .or_insert_with(|| target.display().to_string());
    }
    if rewrites.is_empty() {
        return actions;
    }

    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|mut action| {
            if rewrite_read_action_basename_path(&mut action, &rewrites) {
                changed = true;
            }
            action
        })
        .collect::<Vec<_>>();
    if changed {
        info!("plan_rewrite_active_anchor_basename_file_reads_to_bound_target");
    }
    rewritten
}

fn rewrite_read_action_basename_path(
    action: &mut AgentAction,
    rewrites: &HashMap<String, String>,
) -> bool {
    let (skill, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => return false,
    };
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if !is_read_range_action(skill, obj) {
        return false;
    }
    let Some(current) = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return false;
    };
    let current_path = Path::new(current);
    if current_path.is_absolute() || current_path.components().count() != 1 {
        return false;
    }
    let Some(target) = rewrites.get(&current.to_ascii_lowercase()) else {
        return false;
    };
    if target == current {
        return false;
    }
    obj.insert("path".to_string(), Value::String(target.clone()));
    true
}

#[derive(Debug, Default)]
pub(super) struct ActiveAnchorPlanContext {
    bound_target: Option<String>,
    ordered_entries: Vec<String>,
}

pub(super) fn active_anchor_plan_context(plan_context: Option<&str>) -> ActiveAnchorPlanContext {
    let Some(plan_context) = plan_context else {
        return ActiveAnchorPlanContext::default();
    };
    let mut parsed = ActiveAnchorPlanContext::default();
    let mut in_active_anchor = false;
    for line in plan_context.lines() {
        let line = line.trim();
        if line == "### ACTIVE_EXECUTION_ANCHOR" {
            in_active_anchor = true;
            continue;
        }
        if in_active_anchor && line.starts_with("### ") {
            break;
        }
        if !in_active_anchor {
            continue;
        }
        if let Some(target) = line
            .strip_prefix("followup_bound_target:")
            .or_else(|| line.strip_prefix("observed_bound_target:"))
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            parsed.bound_target = Some(target.to_string());
            continue;
        }
        if let Some(entries) = line
            .strip_prefix("followup_ordered_entries:")
            .or_else(|| line.strip_prefix("observed_ordered_entries:"))
        {
            parsed
                .ordered_entries
                .extend(active_anchor_ordered_entry_targets(entries));
        }
    }
    parsed
}

pub(super) fn active_anchor_ordered_entry_targets(entries: &str) -> Vec<String> {
    entries
        .split(" | ")
        .filter_map(|entry| {
            let (ordinal, target) = entry.trim().split_once(':')?;
            ordinal
                .chars()
                .all(|ch| ch.is_ascii_digit())
                .then_some(target.trim())
        })
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn normalize_path_token_for_anchor_match(path: &str) -> String {
    let mut normalized = path
        .trim()
        .trim_matches('`')
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string();
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

pub(super) fn plain_path_response_items(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| line.trim().trim_matches('`'))
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("{{") && !line.contains("}}"))
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn active_anchor_contains_all_path_items(
    anchor: &ActiveAnchorPlanContext,
    items: &[String],
) -> bool {
    if items.is_empty() || anchor.ordered_entries.is_empty() {
        return false;
    }
    let entry_set = anchor
        .ordered_entries
        .iter()
        .map(|entry| normalize_path_token_for_anchor_match(entry))
        .collect::<HashSet<_>>();
    items.iter().all(|item| {
        field_token_looks_like_locator(item)
            && entry_set.contains(&normalize_path_token_for_anchor_match(item))
    })
}

pub(super) fn find_entries_action_for_selected_anchor_path(
    path: &str,
    bound_target: Option<&str>,
) -> Option<AgentAction> {
    let path = path.trim().trim_matches('`');
    if path.is_empty() || path.contains('\n') || path.contains("{{") {
        return None;
    }
    let path_obj = Path::new(path);
    let basename = path_obj
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path);
    let root = if path_obj.components().count() > 1 || path_obj.is_absolute() {
        path_obj
            .parent()
            .and_then(|parent| parent.to_str())
            .filter(|parent| !parent.is_empty())
            .unwrap_or(".")
            .to_string()
    } else {
        bound_target
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .unwrap_or(".")
            .to_string()
    };
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": basename,
            "target_kind": "file",
            "max_results": 50,
        }),
    })
}

pub(super) fn replace_file_paths_anchor_respond_only_with_find_entries(
    route_result: Option<&RouteResult>,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths)
        || !route.output_contract.requires_content_evidence
    {
        return actions;
    }
    let Some(content) = is_plain_respond_only_plan(&actions) else {
        return actions;
    };
    let items = plain_path_response_items(content);
    let anchor = active_anchor_plan_context(plan_context);
    if !active_anchor_contains_all_path_items(&anchor, &items) {
        return actions;
    }
    let mut rewritten = items
        .iter()
        .filter_map(|item| {
            find_entries_action_for_selected_anchor_path(item, anchor.bound_target.as_deref())
        })
        .collect::<Vec<_>>();
    if rewritten.is_empty() {
        return actions;
    }
    rewritten.push(AgentAction::Respond {
        content: content.trim().to_string(),
    });
    info!(
        "plan_replace_file_paths_anchor_respond_only_with_find_entries entries={}",
        items.len()
    );
    rewritten
}

pub(super) fn replace_scalar_path_anchor_respond_only_with_stat_paths(
    route_result: Option<&RouteResult>,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly)
        || !route.output_contract.requires_content_evidence
    {
        return actions;
    }
    let Some(content) = is_plain_respond_only_plan(&actions) else {
        return actions;
    };
    let items = plain_path_response_items(content);
    if items.len() != 1 {
        return actions;
    }
    let anchor = active_anchor_plan_context(plan_context);
    if !active_anchor_contains_all_path_items(&anchor, &items) {
        return actions;
    }
    info!("plan_replace_scalar_path_anchor_respond_only_with_stat_paths");
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": items,
            "include_missing": true,
        }),
    }]
}

#[derive(Debug, Clone)]
pub(super) struct ParsedConfigChangePreview {
    pub(super) path: String,
    pub(super) field_path: String,
    pub(super) value: Value,
}

pub(super) fn rewrite_config_change_preview_to_config_edit_plan(
    route_result: Option<&RouteResult>,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_has_config_change_contract(route) && !route_has_config_risk_contract(route) {
        return actions;
    }
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || actions.iter().any(|action| {
            action_targets_config_edit(action)
                && !action_is_config_change_preview_observation(action)
        })
        || !actions
            .iter()
            .any(action_is_config_change_preview_observation)
        || actions.iter().any(action_is_obvious_mutation)
    {
        return actions;
    }
    let Some(parsed) = parse_config_change_preview(user_text, route, auto_locator_path) else {
        return actions;
    };
    info!(
        "plan_rewrite_config_change_preview_to_config_edit_plan path={} field={}",
        crate::truncate_for_log(&parsed.path),
        crate::truncate_for_log(&parsed.field_path)
    );
    vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "plan_config_change",
            "path": parsed.path,
            "field_path": parsed.field_path,
            "value": parsed.value,
        }),
    }]
}

pub(super) fn rewrite_config_mutation_to_config_edit_closed_loop(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let has_config_change_recipe = loop_state.execution_recipe.kind
        == crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
        && loop_state.execution_recipe.profile
            == crate::execution_recipe::ExecutionRecipeProfile::ConfigChange;
    let has_planned_mutation = actions.iter().any(action_is_obvious_mutation);
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route_has_config_change_contract(route)
        || !(has_config_change_recipe || has_planned_mutation)
    {
        return actions;
    }
    let Some(parsed) = parse_config_change_preview(user_text, route, auto_locator_path) else {
        return actions;
    };

    info!(
        "plan_rewrite_config_mutation_to_config_edit_closed_loop path={} field={}",
        crate::truncate_for_log(&parsed.path),
        crate::truncate_for_log(&parsed.field_path)
    );
    vec![
        config_edit_change_action("plan_config_change", &parsed),
        config_edit_change_action("apply_config_change", &parsed),
        config_edit_validate_action(&parsed.path),
        config_edit_read_back_action(&parsed),
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ]
}

fn config_edit_change_action(action: &str, parsed: &ParsedConfigChangePreview) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert("action".to_string(), Value::String(action.to_string()));
    args.insert("path".to_string(), Value::String(parsed.path.clone()));
    args.insert(
        "field_path".to_string(),
        Value::String(parsed.field_path.clone()),
    );
    args.insert("value".to_string(), parsed.value.clone());
    args.insert("operation".to_string(), Value::String("set".to_string()));
    if let Some(format) = structured_config_format_for_path(&parsed.path) {
        args.insert("format".to_string(), Value::String(format.to_string()));
    }
    AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: Value::Object(args),
    }
}

fn config_edit_validate_action(path: &str) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert(
        "action".to_string(),
        Value::String("validate_config".to_string()),
    );
    args.insert("path".to_string(), Value::String(path.to_string()));
    if let Some(format) = structured_config_format_for_path(path) {
        args.insert("format".to_string(), Value::String(format.to_string()));
    }
    AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: Value::Object(args),
    }
}

fn config_edit_read_back_action(parsed: &ParsedConfigChangePreview) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert("action".to_string(), Value::String("read_back".to_string()));
    args.insert("path".to_string(), Value::String(parsed.path.clone()));
    args.insert(
        "field_path".to_string(),
        Value::String(parsed.field_path.clone()),
    );
    if let Some(format) = structured_config_format_for_path(&parsed.path) {
        args.insert("format".to_string(), Value::String(format.to_string()));
    }
    AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: Value::Object(args),
    }
}

pub(super) fn parse_config_change_preview(
    _user_text: &str,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<ParsedConfigChangePreview> {
    let field_path = config_change_machine_value(route, &["field_path", "field"])?;
    let value = config_change_machine_json_value(route).or_else(|| {
        config_change_machine_value(route, &["value"]).and_then(parse_config_value_token)
    })?;
    let path = config_change_preview_path(_user_text, route, auto_locator_path)
        .unwrap_or_else(|| "configs/config.toml".to_string());
    Some(ParsedConfigChangePreview {
        path,
        field_path,
        value,
    })
}

pub(super) fn config_change_preview_path(
    _user_text: &str,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    auto_locator_path
        .map(str::trim)
        .filter(|path| path_has_structured_text_extension(path))
        .map(ToString::to_string)
        .or_else(|| route_locator_structured_config_path(route))
}

fn config_change_machine_value(route: &RouteResult, keys: &[&str]) -> Option<String> {
    [
        route.route_reason.as_str(),
        route.resolved_intent.as_str(),
        route.output_contract.locator_hint.as_str(),
    ]
    .into_iter()
    .find_map(|text| {
        keys.iter()
            .find_map(|key| config_change_machine_value_from_text(text, key))
    })
}

fn config_change_machine_value_from_text(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{}=", key.trim());
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')' | '[' | ']'))
        .find_map(|part| {
            let raw = part.trim().strip_prefix(&prefix)?;
            let value = raw.trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
            (!value.is_empty()).then(|| value.to_string())
        })
}

fn config_change_machine_json_value(route: &RouteResult) -> Option<Value> {
    config_change_machine_value(route, &["value_json", "json_value"])
        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
}

pub(super) fn route_locator_structured_config_path(route: &RouteResult) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() || !path_has_structured_text_extension(hint) {
        return None;
    }
    matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    )
    .then(|| hint.to_string())
}

pub(super) fn rewrite_structured_scalar_field_read_plan_to_read_field(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    allow_route_resolved_intent_selector: bool,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let identifier_presence_contract = route_reason_has_structural_marker(
        route,
        "structured_identifier_presence_requires_content_evidence",
    );
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract_marker_is(crate::OutputSemanticKind::StructuredKeys)
        || route.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment)
        || (!identifier_presence_contract
            && actions.iter().any(action_is_structured_config_validation))
        || actions.iter().any(action_is_structured_scalar_field_read)
        || (!actions.iter().any(action_observes_structured_source)
            && !(identifier_presence_contract
                && actions.iter().any(
                    super::super::planning_path_metadata::planned_action_is_single_path_metadata_facts,
                )))
        || actions.iter().any(|action| {
            !action_observes_structured_source(action)
                && !(identifier_presence_contract
                    && super::super::planning_path_metadata::planned_action_is_single_path_metadata_facts(
                        action,
                    ))
                && !matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. }
                        | AgentAction::Respond { .. }
                        | AgentAction::Think { .. }
                )
        })
    {
        return actions;
    }
    let Some(path) = structured_scalar_field_read_target_path(route, auto_locator_path, &actions)
    else {
        return actions;
    };
    let Some(field_path) = structured_scalar_field_selector(
        route,
        user_text,
        allow_route_resolved_intent_selector,
        plan_context,
        Some(&path),
    )
    .or_else(|| {
        structured_scalar_field_selector_from_structural_candidates(
            state,
            route,
            user_text,
            allow_route_resolved_intent_selector,
            plan_context,
            &path,
        )
    }) else {
        return actions;
    };
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
    ) && !actions.iter().any(action_is_readonly_config_observation)
    {
        return actions;
    }

    let (path, field_path) =
        resolve_structured_scalar_read_target_and_field(state, route, &path, &field_path);
    info!(
        "plan_rewrite_structured_scalar_field_read_to_config_basic path={} field={}",
        crate::truncate_for_log(&path),
        crate::truncate_for_log(&field_path)
    );
    vec![config_basic_read_field_action(path, field_path)]
}

pub(super) fn rewrite_scalar_candidate_respond_to_structured_field_read(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        || !loop_state.has_tool_or_skill_output
        || !route_allows_structured_scalar_candidate_field_recovery(route)
    {
        return actions;
    }
    let Some(content) = is_plain_respond_only_plan(&actions).map(str::trim) else {
        return actions;
    };
    if content.is_empty() || content.contains("{{") || content.lines().count() > 1 {
        return actions;
    }
    let Some(path) = structured_scalar_field_read_target_path(route, auto_locator_path, &actions)
    else {
        return actions;
    };
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, &path);
    let Some(value) = parse_structured_file_value(&current) else {
        return actions;
    };
    let Some(field_path) = unique_structured_scalar_field_path_for_candidate(&value, content)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_scalar_candidate_respond_to_structured_field_read path={} field={}",
        crate::truncate_for_log(&path),
        crate::truncate_for_log(&field_path)
    );
    vec![config_basic_read_field_action(path, field_path)]
}

pub(super) fn route_allows_structured_scalar_candidate_field_recovery(route: &RouteResult) -> bool {
    [
        "active_clarify_fast_path_scalar_field_value_contract_repair",
        "structured_field_selector_requires_scalar_value",
        "structured_keys_scalar_response_requires_field_value",
        "single_path_field_extraction_semantic_kind_none_is_valid",
        "contract_valid_minor_repair_fields_only",
    ]
    .iter()
    .any(|marker| route_reason_has_structural_marker(route, marker))
}

pub(super) fn unique_structured_scalar_field_path_for_candidate(
    value: &Value,
    candidate: &str,
) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return None;
    }
    let mut matches = Vec::new();
    collect_structured_scalar_field_paths_for_candidate(value, "", candidate, &mut matches);
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

pub(super) fn collect_structured_scalar_field_paths_for_candidate(
    value: &Value,
    prefix: &str,
    candidate: &str,
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if !schema_field_token_is_valid(key) {
                    continue;
                }
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if structured_scalar_value_matches_candidate(child, candidate) {
                    out.push(path.clone());
                }
                collect_structured_scalar_field_paths_for_candidate(child, &path, candidate, out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let path = if prefix.is_empty() {
                    idx.to_string()
                } else {
                    format!("{prefix}.{idx}")
                };
                if structured_scalar_value_matches_candidate(child, candidate) {
                    out.push(path.clone());
                }
                collect_structured_scalar_field_paths_for_candidate(child, &path, candidate, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

pub(super) fn structured_scalar_value_matches_candidate(value: &Value, candidate: &str) -> bool {
    match value {
        Value::String(actual) => {
            actual == candidate
                || parse_answer_candidate_value(candidate)
                    .as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|parsed| parsed == actual)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => parse_answer_candidate_value(candidate)
            .as_ref()
            .is_some_and(|candidate_value| candidate_value == value),
        Value::Array(_) | Value::Object(_) => false,
    }
}

pub(super) fn add_prior_structured_text_field_read_for_scalar_compare(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        || structured_scalar_observation_units(&actions) != 1
        || executed_structured_scalar_observation_units(loop_state) > 0
    {
        return actions;
    }

    let current_scalar_paths = planned_structured_scalar_read_paths(&actions)
        .into_iter()
        .map(|path| resolve_workspace_path(&state.skill_rt.workspace_root, &path))
        .collect::<Vec<_>>();
    let prior_paths = executed_structured_text_read_paths(loop_state);
    for prior_path in prior_paths.into_iter().rev() {
        let resolved_prior = resolve_workspace_path(&state.skill_rt.workspace_root, &prior_path);
        if current_scalar_paths
            .iter()
            .any(|path| same_existing_or_display_path(path, &resolved_prior))
        {
            continue;
        }
        let Some(action) = structured_scalar_read_action_for_target(
            state,
            route,
            user_text,
            resolved_prior.to_string_lossy().as_ref(),
        )
        .or_else(|| {
            plan_context.and_then(|context| {
                structured_scalar_read_action_for_target(
                    state,
                    route,
                    context,
                    resolved_prior.to_string_lossy().as_ref(),
                )
            })
        }) else {
            continue;
        };
        let Some((skill, args)) = planned_call_subject_and_args(&action) else {
            continue;
        };
        if !crate::evidence_policy::capability_ref_action_policy_for_route(Some(route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
        {
            continue;
        }

        let mut rewritten = Vec::with_capacity(actions.len() + 1);
        rewritten.push(action);
        rewritten.extend(actions);
        refresh_synthesize_refs_from_prior_observations(&mut rewritten);
        info!(
            "plan_add_prior_structured_text_field_read_for_scalar_compare path={}",
            crate::truncate_for_log(&resolved_prior.display().to_string())
        );
        return rewritten;
    }

    actions
}

pub(super) fn planned_call_subject_and_args(action: &AgentAction) -> Option<(&str, &Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        _ => None,
    }
}

pub(super) fn planned_structured_scalar_read_paths(actions: &[AgentAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| {
            action_is_structured_scalar_field_read(action)
                .then(|| planned_call_subject_and_args(action))
                .flatten()
                .and_then(|(_, args)| args.get("path").and_then(Value::as_str))
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
        })
        .collect()
}

pub(super) fn executed_structured_text_read_paths(loop_state: &LoopState) -> Vec<String> {
    let mut paths = Vec::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok()
            || !(step.skill.eq_ignore_ascii_case("fs_basic")
                || step.skill.eq_ignore_ascii_case("system_basic"))
        {
            continue;
        }
        let Some(value) = step
            .output
            .as_deref()
            .and_then(|output| serde_json::from_str::<Value>(output).ok())
            .map(step_output_machine_payload)
        else {
            continue;
        };
        let action = value
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let is_text_read = (step.skill.eq_ignore_ascii_case("fs_basic")
            && action.eq_ignore_ascii_case("read_text_range"))
            || (step.skill.eq_ignore_ascii_case("system_basic")
                && action.eq_ignore_ascii_case("read_range"));
        if !is_text_read {
            continue;
        }
        let Some(path) = value
            .get("resolved_path")
            .or_else(|| value.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .filter(|path| path_has_structured_document_extension(path))
        else {
            continue;
        };
        if !paths
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(path))
        {
            paths.push(path.to_string());
        }
    }
    paths
}

pub(super) fn refresh_synthesize_refs_from_prior_observations(actions: &mut [AgentAction]) {
    for idx in 0..actions.len() {
        let refs = observation_action_evidence_refs(&actions[..idx]);
        if refs.is_empty() {
            continue;
        }
        if let AgentAction::SynthesizeAnswer { evidence_refs } = &mut actions[idx] {
            *evidence_refs = refs;
        }
    }
}

pub(super) fn resolve_structured_scalar_read_target_and_field(
    state: &AppState,
    route: &RouteResult,
    path: &str,
    field_path: &str,
) -> (String, String) {
    let fields = vec![field_path.to_string()];
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, path);
    if structured_file_has_all_fields(&current, &fields)
        && route_locator_targets_current_path(route, &state.skill_rt.workspace_root, &current)
    {
        return (path.to_string(), field_path.to_string());
    }
    if let Some((target, rewritten_fields)) =
        resolve_cargo_workspace_package_fields(&state.skill_rt.workspace_root, &current, &fields)
    {
        if rewritten_fields.len() == 1 {
            return (
                target.display().to_string(),
                rewritten_fields[0].to_string(),
            );
        }
    }
    if structured_file_has_all_fields(&current, &fields) {
        return (path.to_string(), field_path.to_string());
    }
    if !route_allows_structured_candidate_read_target_repair(route) {
        return (path.to_string(), field_path.to_string());
    }
    find_structured_field_candidate(
        &state.skill_rt.workspace_root,
        &current,
        &fields,
        state.skill_rt.locator_scan_max_files,
    )
    .map(|replacement| (replacement.display().to_string(), field_path.to_string()))
    .unwrap_or_else(|| (path.to_string(), field_path.to_string()))
}

pub(super) fn route_locator_targets_current_path(
    route: &RouteResult,
    workspace_root: &Path,
    current: &Path,
) -> bool {
    route_locator_structured_config_path(route)
        .map(|locator| resolve_workspace_path(workspace_root, &locator))
        .is_some_and(|locator| same_existing_or_display_path(&locator, current))
}
