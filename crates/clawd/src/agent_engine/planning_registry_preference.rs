use claw_core::skill_registry::PrimaryFallbackRole;
use serde_json::Value;
use std::path::Path;

use super::planning_actions::planned_action_skill_name;
use crate::{AgentAction, AppState, RouteResult};

fn route_registry_preference_tags(route_result: &RouteResult) -> Vec<String> {
    route_capability_registry_tags(route_result)
}

fn route_capability_registry_tags(route_result: &RouteResult) -> Vec<String> {
    let mut tags = Vec::new();
    for token in crate::machine_capability_ref::route_capability_ref_tokens(route_result) {
        tags.push(token.clone());
        if let Some((namespace, action)) = token.split_once('.') {
            tags.push(format!("{namespace}_{action}"));
            append_capability_alias_tags(&mut tags, namespace, action);
        }
    }
    tags.sort();
    tags.dedup();
    tags
}

fn append_capability_alias_tags(tags: &mut Vec<String>, namespace: &str, action: &str) {
    match namespace {
        "archive" if matches!(action, "list" | "read" | "pack" | "unpack") => {
            tags.push(format!("archive_{action}"));
        }
        "config" => append_config_capability_alias_tags(tags, action),
        "database" | "db" | "sqlite" => append_database_capability_alias_tags(tags, action),
        "docker" => append_docker_capability_alias_tags(tags, action),
        "filesystem" | "fs_basic" => append_filesystem_capability_alias_tags(tags, action),
        "package" | "package_manager" if action == "detect" => {
            tags.push("package.detect_manager".to_string());
        }
        "process" if matches!(action, "ps" | "status" | "runtime_status") => {
            tags.push("service_status".to_string());
            tags.push("runtime_health".to_string());
        }
        "system" | "system_basic" if matches!(action, "runtime_status" | "status") => {
            tags.push("service_status".to_string());
            tags.push("runtime_health".to_string());
            tags.push("system_health".to_string());
        }
        _ => {}
    }
}

fn append_filesystem_capability_alias_tags(tags: &mut Vec<String>, action: &str) {
    match action {
        "stat_path" | "stat_paths" => {
            tags.push("path_facts".to_string());
            tags.push("existence_with_path".to_string());
        }
        "list_dir" | "list_entries" => {
            tags.push("directory_inventory".to_string());
            tags.push("directory_names".to_string());
            tags.push("file_names".to_string());
        }
        "find_entries" | "find_files" | "find_paths" => {
            tags.push("file_search".to_string());
            tags.push("file_paths".to_string());
        }
        "grep_text" | "search_text" => tags.push("content_search".to_string()),
        "read_file" | "read_text" | "read_text_range" | "read_range" => {
            tags.push("bounded_file_read".to_string());
        }
        "compare_paths" => {
            tags.push("path_compare".to_string());
            tags.push("quantity_comparison".to_string());
        }
        "write_file" | "write_text" | "append_file" | "append_text" | "make_dir" | "create_dir"
        | "remove_path" | "delete_path" => {
            tags.push("file_write".to_string());
        }
        _ => {}
    }
}

fn append_config_capability_alias_tags(tags: &mut Vec<String>, action: &str) {
    match action {
        "validate" | "validate_config" | "validate_after_change" => {
            tags.push("config_validation".to_string());
        }
        "guard_config" | "guard_after_change" | "guard_rustclaw_config" => {
            tags.push("config_guard".to_string());
            tags.push("config_risk_assessment".to_string());
        }
        "list_keys" => tags.push("structured_keys".to_string()),
        "read_field" | "read_fields" | "read_back" => tags.push("field_read".to_string()),
        "plan_change"
        | "plan_config_change"
        | "apply_change"
        | "apply_config_change"
        | "write_field"
        | "set_field" => tags.push("config_mutation".to_string()),
        _ => {}
    }
}

fn append_database_capability_alias_tags(tags: &mut Vec<String>, action: &str) {
    match action {
        "query" | "sqlite_query" => tags.push("sqlite_query".to_string()),
        "list" | "list_tables" | "tables" => {
            tags.push("sqlite_table_listing".to_string());
            tags.push("sqlite_table_names_only".to_string());
        }
        "schema_version" => tags.push("sqlite_schema_version".to_string()),
        "kind" | "database_kind" => tags.push("sqlite_database_kind_judgment".to_string()),
        _ => {}
    }
}

fn append_docker_capability_alias_tags(tags: &mut Vec<String>, action: &str) {
    match action {
        "ps" | "list" | "list_containers" => tags.push("docker.list_containers".to_string()),
        "image" | "images" | "list_images" => tags.push("docker.list_images".to_string()),
        "log" | "logs" | "read" | "read_logs" => tags.push("docker.read_logs".to_string()),
        "inspect" | "restart" | "start" | "stop" => tags.push("docker.lifecycle".to_string()),
        _ => {}
    }
}

pub(super) fn registry_preferred_skill_names_for_route(
    state: &AppState,
    route_result: &RouteResult,
) -> Vec<String> {
    let Some(registry) = state.get_skills_registry() else {
        return Vec::new();
    };
    let enabled_skills = state.get_skills_list();
    let route_tags = route_registry_preference_tags(route_result);
    let mut preferred = if route_tags.is_empty() {
        Vec::new()
    } else {
        registry
            .enabled_names()
            .into_iter()
            .filter(|name| enabled_skills.is_empty() || enabled_skills.contains(name))
            .filter(|name| {
                registry.get(name).is_some_and(|entry| {
                    entry.preferred_over_run_cmd
                        && entry.semantic_tags.iter().any(|tag| {
                            let tag = tag.trim();
                            route_tags
                                .iter()
                                .any(|route_tag| tag.eq_ignore_ascii_case(route_tag.as_str()))
                        })
                })
            })
            .collect::<Vec<_>>()
    };
    if route_targets_log_analysis(route_result) {
        preferred.extend(
            registry
                .enabled_names()
                .into_iter()
                .filter(|name| enabled_skills.is_empty() || enabled_skills.contains(name))
                .filter(|name| name.eq_ignore_ascii_case("log_analyze")),
        );
    }
    preferred.sort_by_key(|name| name.to_ascii_lowercase());
    preferred.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    preferred
}

fn route_targets_log_analysis(route_result: &RouteResult) -> bool {
    let contract = route_result.effective_output_contract();
    contract.requires_content_evidence
        && !contract.delivery_required
        && route_result.output_contract_marker_is(crate::OutputSemanticKind::ContentExcerptSummary)
        && path_targets_log_artifact(&route_result.output_contract.locator_hint)
}

fn path_targets_log_artifact(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    let parsed = Path::new(path);
    parsed
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
        || parsed.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|part| part.eq_ignore_ascii_case("logs"))
        })
}

#[cfg(test)]
pub(super) fn registry_preferred_skill_matches_route(
    state: &AppState,
    route_result: &RouteResult,
) -> bool {
    !registry_preferred_skill_names_for_route(state, route_result).is_empty()
}

pub(super) fn actions_use_ad_hoc_command_without_route_preferred_skill(
    state: &AppState,
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    let preferred_skills = registry_preferred_skill_names_for_route(state, route_result);
    if preferred_skills.is_empty() {
        return false;
    }
    if structured_bounded_log_slice_plan_satisfies_route(route_result, actions) {
        return false;
    }
    if actions.iter().any(|action| {
        planned_action_skill_name(action).is_some_and(|skill| {
            let canonical = state.resolve_canonical_skill_name(skill);
            preferred_skills
                .iter()
                .any(|preferred| preferred.eq_ignore_ascii_case(&canonical))
        }) || action_satisfies_structured_key_listing_contract(route_result, action)
    }) {
        return false;
    }
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action) else {
            return false;
        };
        let canonical = state.resolve_canonical_skill_name(skill);
        if canonical.eq_ignore_ascii_case("run_cmd")
            && action_has_internal_literal_command_marker(action)
        {
            return false;
        }
        if canonical.eq_ignore_ascii_case("run_cmd")
            && action_has_runtime_async_job_start_marker(action)
        {
            return false;
        }
        if action_satisfies_structured_key_listing_contract(route_result, action) {
            return false;
        }
        action_uses_generic_fallback_capability_for_preferred_route(
            state,
            route_result,
            &canonical,
            action,
        )
    })
}

fn structured_bounded_log_slice_plan_satisfies_route(
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !route_targets_log_analysis(route_result) {
        return false;
    }
    let mut saw_bounded_log_read = false;
    let mut saw_executable = false;
    let mut saw_terminal_synthesis = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => {
                if !skill.eq_ignore_ascii_case("fs_basic") {
                    return false;
                }
                let Some(action_name) = args.get("action").and_then(Value::as_str).map(str::trim)
                else {
                    return false;
                };
                saw_executable = true;
                match action_name {
                    "list_dir" | "find_entries" => {}
                    "read_text_range" if read_text_range_uses_bounded_slice(args) => {
                        saw_bounded_log_read = true;
                    }
                    _ => return false,
                }
            }
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. } => {
                saw_terminal_synthesis = true;
            }
            AgentAction::CallCapability { .. } | AgentAction::Think { .. } => return false,
        }
    }
    saw_executable && saw_bounded_log_read && saw_terminal_synthesis
}

fn read_text_range_uses_bounded_slice(args: &Value) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
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

fn action_satisfies_structured_key_listing_contract(
    route_result: &RouteResult,
    action: &AgentAction,
) -> bool {
    if !action_is_structured_key_listing(action) {
        return false;
    }
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::StructuredKeys) {
        return true;
    }
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::FileNames) {
        return action_structured_key_listing_path(action)
            .or_else(|| {
                let hint = route_result.output_contract.locator_hint.trim();
                (!hint.is_empty()).then_some(hint)
            })
            .is_some_and(path_has_structured_document_extension);
    }
    false
}

fn action_is_structured_key_listing(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let Some(action_name) = args.get("action").and_then(Value::as_str) else {
                return false;
            };
            (skill.eq_ignore_ascii_case("config_basic")
                && action_name.eq_ignore_ascii_case("list_keys"))
                || (skill.eq_ignore_ascii_case("system_basic")
                    && action_name.eq_ignore_ascii_case("structured_keys"))
        }
        AgentAction::CallCapability { .. } => false,
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn action_structured_key_listing_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. }
            if action_is_structured_key_listing(action) =>
        {
            args.get("path").and_then(Value::as_str).map(str::trim)
        }
        _ => None,
    }
    .filter(|path| !path.is_empty())
}

pub(super) fn path_has_structured_document_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim().to_ascii_lowercase())
        .is_some_and(|extension| matches!(extension.as_str(), "json" | "toml" | "yaml" | "yml"))
}

fn action_uses_generic_fallback_capability_for_preferred_route(
    state: &AppState,
    route_result: &RouteResult,
    canonical_skill_name: &str,
    action: &AgentAction,
) -> bool {
    if route_targets_log_analysis(route_result)
        && canonical_skill_name.eq_ignore_ascii_case("fs_basic")
        && action_is_generic_file_observation(action)
    {
        return true;
    }
    if !canonical_skill_name.eq_ignore_ascii_case("run_cmd") {
        return false;
    }

    state.get_skills_registry().is_some_and(|registry| {
        registry.get(canonical_skill_name).is_some_and(|entry| {
            matches!(
                entry.primary_fallback_role,
                Some(PrimaryFallbackRole::Fallback)
            )
        })
    })
}

fn action_is_generic_file_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args
            .get("action")
            .and_then(Value::as_str)
            .is_some_and(|action| {
                matches!(
                    action.trim(),
                    "read_text_range" | "read_range" | "grep_text" | "list_dir" | "find_entries"
                )
            }),
        AgentAction::CallCapability { capability, .. } => matches!(
            capability.trim(),
            "filesystem.read_text_range"
                | "fs_basic.read_text_range"
                | "filesystem.grep_text"
                | "fs_basic.grep_text"
                | "filesystem.list_entries"
                | "fs_basic.list_dir"
                | "filesystem.find_entries"
                | "fs_basic.find_entries"
        ),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn action_has_internal_literal_command_marker(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args
            .get(super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        AgentAction::CallCapability { .. } => false,
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

fn action_has_runtime_async_job_start_marker(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            args.get(super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG)
                .and_then(Value::as_str)
                == Some("async_job_protocol")
                && args.get("async_start").and_then(Value::as_bool) == Some(true)
        }
        AgentAction::CallCapability { .. } => false,
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}
