use super::*;

pub(super) fn rewrite_rustclaw_config_validation_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| {
            let Some((path, format, profile)) =
                config_validation_action_target(&action, route_result, auto_locator_path)
                    .map(|(path, format, profile)| (path, format, Some(profile)))
                    .or_else(|| {
                        plain_rustclaw_main_config_validation_action_target(
                            &action,
                            route_result,
                            auto_locator_path,
                        )
                        .map(|(path, format)| (path, format, None))
                    })
            else {
                return action;
            };
            if profile == Some(ConfigValidationProfile::SyntaxOnly) {
                return action;
            }
            let candidate = config_basic_guard_action(path, format);
            if !planned_action_allowed_by_current_contract(route_result, &candidate) {
                return action;
            }
            let candidate_path = match &candidate {
                AgentAction::CallTool { args, .. } => {
                    args.get("path").and_then(Value::as_str).unwrap_or_default()
                }
                _ => "",
            };
            info!(
                "plan_rewrite_rustclaw_config_validation_to_guard path={}",
                crate::truncate_for_log(candidate_path)
            );
            candidate
        })
        .collect()
}

pub(super) fn planned_action_allowed_by_current_contract(
    route_result: Option<&RouteResult>,
    action: &AgentAction,
) -> bool {
    let Some(route) = route_result else {
        return true;
    };
    let Some((skill, args)) = planned_execution_action_ref(action) else {
        return true;
    };
    crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        skill,
        args,
    )
    .is_some_and(|policy| policy.is_allowed())
}

pub(super) fn repair_guard_config_default_path_for_invalid_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallTool { tool, args } => {
                let args = repair_guard_config_args_for_invalid_locator(
                    &tool,
                    args,
                    route_result,
                    auto_locator_path,
                );
                AgentAction::CallTool { tool, args }
            }
            AgentAction::CallSkill { skill, args } => {
                let args = repair_guard_config_args_for_invalid_locator(
                    &skill,
                    args,
                    route_result,
                    auto_locator_path,
                );
                AgentAction::CallSkill { skill, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn repair_guard_config_args_for_invalid_locator(
    skill: &str,
    args: Value,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Value {
    let Some(action_name) = args.get("action").and_then(Value::as_str).map(str::trim) else {
        return args;
    };
    let is_guard_action = (skill.eq_ignore_ascii_case("config_edit")
        && action_name.eq_ignore_ascii_case("guard_config"))
        || (skill.eq_ignore_ascii_case("config_basic")
            && action_name.eq_ignore_ascii_case("guard_rustclaw_config"))
        || skill.eq_ignore_ascii_case("config_guard");
    if !is_guard_action {
        return args;
    }
    let should_repair = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .is_none_or(|path| {
            !is_rustclaw_config_guard_path(path) && !path_has_structured_text_extension(path)
        });
    if !should_repair {
        return args;
    }
    let path = route_result
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .unwrap_or("configs/config.toml");
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.insert("path".to_string(), Value::String(path.to_string()));
    obj.entry("format".to_string())
        .or_insert_with(|| Value::String("toml".to_string()));
    Value::Object(obj)
}

pub(super) fn rewrite_rustclaw_config_risk_assessment_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_none_or(|route| {
        route.output_contract.semantic_kind != crate::OutputSemanticKind::ConfigRiskAssessment
    }) || actions.iter().any(is_config_basic_guard_action)
    {
        return actions;
    }
    let Some(path) =
        rustclaw_config_risk_assessment_target(route_result, auto_locator_path, &actions)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_rustclaw_config_risk_assessment_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    vec![config_basic_guard_action(path, Some("toml".to_string()))]
}

pub(super) fn rustclaw_config_risk_assessment_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    actions
        .iter()
        .filter_map(planned_config_risk_observation_path)
        .find(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .map(ToString::to_string)
}

pub(super) fn planned_config_risk_observation_path(action: &AgentAction) -> Option<&str> {
    planned_structured_config_observation_path(action)
        .or_else(|| planned_bounded_file_read_path(action))
}

pub(super) fn rewrite_rustclaw_main_config_excerpt_read_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    let Some(path) =
        rustclaw_main_config_excerpt_guard_target(route_result, auto_locator_path, &actions)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_rustclaw_main_config_excerpt_read_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    vec![config_basic_guard_action(path, Some("toml".to_string()))]
}

pub(super) fn rustclaw_main_config_excerpt_guard_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ContentExcerptSummary
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
    {
        return None;
    }
    actions
        .iter()
        .filter_map(planned_broad_rustclaw_main_config_read_path)
        .find(|path| is_rustclaw_main_config_path(path))
        .or_else(|| {
            let has_broad_main_config_read = actions
                .iter()
                .any(|action| planned_broad_config_read_without_path(action));
            has_broad_main_config_read.then(|| {
                auto_locator_path
                    .map(str::trim)
                    .filter(|path| is_rustclaw_main_config_path(path))
            })?
        })
        .or_else(|| {
            let has_broad_main_config_read = actions
                .iter()
                .any(|action| planned_broad_config_read_without_path(action));
            has_broad_main_config_read.then(|| {
                let hint = route.output_contract.locator_hint.trim();
                is_rustclaw_main_config_path(hint).then_some(hint)
            })?
        })
        .map(ToString::to_string)
}

pub(super) fn planned_broad_rustclaw_main_config_read_path(action: &AgentAction) -> Option<&str> {
    planned_bounded_file_read_path(action)
        .filter(|path| is_rustclaw_main_config_path(path))
        .filter(|_| planned_broad_config_excerpt_read(action))
}

pub(super) fn planned_broad_config_read_without_path(action: &AgentAction) -> bool {
    planned_bounded_file_read_path(action).is_none() && planned_broad_config_excerpt_read(action)
}

pub(super) fn planned_broad_config_excerpt_read(action: &AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Think { .. } => return false,
    };
    if !action_observes_bounded_file_content(action) {
        return false;
    }
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if mode.eq_ignore_ascii_case("tail") || mode.eq_ignore_ascii_case("last") {
        return false;
    }
    let n = args
        .get("n")
        .or_else(|| args.get("line_count"))
        .or_else(|| args.get("count"))
        .or_else(|| args.get("limit"))
        .and_then(parse_positive_usize);
    if n.is_some_and(|value| value < 80) {
        return false;
    }
    let start_line = args
        .get("start_line")
        .or_else(|| args.get("line_start"))
        .and_then(parse_i64_value);
    if start_line.is_some_and(|line| line > 1) {
        return false;
    }
    let end_line = args
        .get("end_line")
        .or_else(|| args.get("line_end"))
        .and_then(parse_i64_value);
    if let (Some(start), Some(end)) = (start_line, end_line) {
        if end >= start && (end - start + 1) < 80 {
            return false;
        }
    } else if let Some(end) = end_line {
        if end < 80 {
            return false;
        }
    }
    let max_bytes = args.get("max_bytes").and_then(parse_positive_usize);
    if max_bytes.is_some_and(|value| value < 4096) {
        return false;
    }
    true
}

pub(super) fn rewrite_invalid_rustclaw_config_section_field_reads_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    let Some((path, format)) = invalid_rustclaw_config_section_field_read_target(
        route_result,
        auto_locator_path,
        &actions,
    ) else {
        return actions;
    };
    info!(
        "plan_rewrite_invalid_rustclaw_config_section_field_read_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    vec![config_basic_guard_action(path, format)]
}

pub(super) fn prefer_config_basic_guard_for_rustclaw_config_actions(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| {
            let Some((path, format)) =
                config_guard_action_target_path(&action, route_result, auto_locator_path)
            else {
                return action;
            };
            let candidate = config_basic_guard_action(path, format);
            if planned_action_allowed_by_current_contract(route_result, &candidate) {
                candidate
            } else {
                action
            }
        })
        .collect()
}

pub(super) fn config_basic_guard_action(path: String, format: Option<String>) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert(
        "action".to_string(),
        Value::String("guard_rustclaw_config".to_string()),
    );
    args.insert("path".to_string(), Value::String(path));
    if let Some(format) = format {
        args.insert("format".to_string(), Value::String(format));
    }
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: Value::Object(args),
    }
}

pub(super) fn prefer_route_locator_for_rustclaw_config_action_paths(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(locator) = route_result
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|path| !path.is_empty() && is_rustclaw_config_guard_path(path))
    else {
        return actions;
    };
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallTool { tool, args } => AgentAction::CallTool {
                args: prefer_route_locator_for_config_args(&tool, args, locator),
                tool,
            },
            AgentAction::CallSkill { skill, args } => AgentAction::CallSkill {
                args: prefer_route_locator_for_config_args(&skill, args, locator),
                skill,
            },
            other => other,
        })
        .collect()
}

pub(super) fn prefer_route_locator_for_config_args(
    skill: &str,
    args: Value,
    locator: &str,
) -> Value {
    if !matches!(
        skill.trim().to_ascii_lowercase().as_str(),
        "config_basic" | "config_edit" | "config_guard"
    ) {
        return args;
    }
    let Some(path) = args.get("path").and_then(Value::as_str).map(str::trim) else {
        return args;
    };
    if !is_rustclaw_config_guard_path(path) {
        return args;
    }
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.insert("path".to_string(), Value::String(locator.to_string()));
    if skill.trim().eq_ignore_ascii_case("config_basic")
        && obj
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|action| action.eq_ignore_ascii_case("guard_rustclaw_config"))
    {
        obj.entry("format".to_string())
            .or_insert_with(|| Value::String("toml".to_string()));
    }
    Value::Object(obj)
}

pub(super) fn config_guard_action_target_path(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>)> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_runtime_guard = (skill.eq_ignore_ascii_case("config_edit")
        && action_name.eq_ignore_ascii_case("guard_config"))
        || skill.eq_ignore_ascii_case("config_guard");
    if !is_runtime_guard {
        return None;
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| is_rustclaw_config_guard_path(path))
        })?;
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|format| !format.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format))
}

pub(super) fn invalid_rustclaw_config_section_field_read_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<(String, Option<String>)> {
    actions.iter().find_map(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            _ => return None,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let is_field_read = (skill.eq_ignore_ascii_case("config_basic")
            && matches!(action_name, "read_field" | "read_fields"))
            || (skill.eq_ignore_ascii_case("system_basic")
                && matches!(action_name, "extract_field" | "extract_fields"));
        if !is_field_read || !config_field_args_are_section_headers(args) {
            return None;
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .or_else(|| {
                auto_locator_path
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
            })
            .or_else(|| {
                route_result
                    .map(|route| route.output_contract.locator_hint.trim())
                    .filter(|path| !path.is_empty())
            })?;
        if !is_rustclaw_main_config_path(path) {
            return None;
        }
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| Some("toml".to_string()));
        Some((path.to_string(), format))
    })
}

pub(super) fn config_field_args_are_section_headers(args: &Value) -> bool {
    let fields = args
        .get("field_paths")
        .or_else(|| args.get("fields"))
        .map(config_field_selector_list)
        .filter(|fields| !fields.is_empty())
        .or_else(|| {
            args.get("field_path")
                .or_else(|| args.get("field"))
                .or_else(|| args.get("key"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(|field| vec![field.to_string()])
        })
        .unwrap_or_default();
    fields.len() >= 2
        && fields
            .iter()
            .all(|field| config_field_is_section_header(field))
}

pub(super) fn config_field_selector_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToString::to_string)
            .collect(),
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn config_field_is_section_header(field: &str) -> bool {
    let field = field.trim();
    field.len() > 2
        && field.starts_with('[')
        && field.ends_with(']')
        && !field[1..field.len() - 1].trim().is_empty()
}

pub(super) fn is_config_guard_action(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    (skill.eq_ignore_ascii_case("config_edit") && action_name.eq_ignore_ascii_case("guard_config"))
        || (skill.eq_ignore_ascii_case("config_basic")
            && action_name.eq_ignore_ascii_case("guard_rustclaw_config"))
        || skill.eq_ignore_ascii_case("config_guard")
}

pub(super) fn is_config_basic_guard_action(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("guard_rustclaw_config")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigValidationProfile {
    SyntaxOnly,
    RustClawSemanticGuard,
}

pub(super) fn parse_config_validation_profile(value: &str) -> Option<ConfigValidationProfile> {
    match value.trim().to_ascii_lowercase().as_str() {
        "syntax_only" => Some(ConfigValidationProfile::SyntaxOnly),
        "rustclaw_semantic_guard" => Some(ConfigValidationProfile::RustClawSemanticGuard),
        _ => None,
    }
}

pub(super) fn config_validation_profile_from_args(args: &Value) -> Option<ConfigValidationProfile> {
    args.get("validation_profile")
        .and_then(Value::as_str)
        .and_then(parse_config_validation_profile)
        .or_else(|| {
            args.get("_clawd_validation")
                .and_then(Value::as_object)
                .and_then(|obj| obj.get("validation_profile"))
                .and_then(Value::as_str)
                .and_then(parse_config_validation_profile)
        })
}

pub(super) fn config_validation_action_target(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>, ConfigValidationProfile)> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    let profile = config_validation_profile_from_args(args)?;
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_validation = (skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("validate"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("validate_structured"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && action_name.eq_ignore_ascii_case("validate_config"));
    if !is_validation {
        return None;
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| !path.is_empty())
        })?;
    if !is_rustclaw_main_config_path(path) {
        return None;
    }
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format, profile))
}

pub(super) fn plain_rustclaw_main_config_validation_action_target(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>)> {
    if route_result.is_none_or(|route| !route.output_contract.requires_content_evidence) {
        return None;
    }
    if route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigValidation
    }) {
        return None;
    }
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if config_validation_profile_from_args(args).is_some() {
        return None;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_validation = (skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("validate"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("validate_structured"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && action_name.eq_ignore_ascii_case("validate_config"));
    if !is_validation {
        return None;
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| !path.is_empty())
        })?;
    if !is_rustclaw_main_config_path(path) {
        return None;
    }
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format))
}

pub(super) fn is_rustclaw_main_config_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").trim().to_ascii_lowercase();
    normalized == "configs/config.toml"
        || normalized.ends_with("/configs/config.toml")
        || normalized == "config.toml"
}

pub(super) fn is_rustclaw_config_guard_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").trim().to_ascii_lowercase();
    if is_rustclaw_main_config_path(&normalized) {
        return true;
    }
    let relative_configs_path = normalized.starts_with("configs/") && normalized.ends_with(".toml");
    let absolute_configs_path = normalized.contains("/configs/") && normalized.ends_with(".toml");
    relative_configs_path || absolute_configs_path
}
