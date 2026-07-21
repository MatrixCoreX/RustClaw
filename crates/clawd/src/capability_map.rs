use std::collections::{BTreeMap, BTreeSet};

use crate::{skill_availability, AppState, ClaimedTask};
use claw_core::skill_registry::{
    PlannerCapabilityKind, PlannerCapabilityMapping, SkillRegistryEntry,
};

const UNGROUPED_CAPABILITY_TOKEN: &str = "ungrouped";

fn registry_group_token(entry: &SkillRegistryEntry) -> Option<String> {
    entry
        .group
        .as_deref()
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_ascii_lowercase)
}

fn classify_skill(state: &AppState, skill: &str) -> String {
    state
        .get_skills_registry()
        .and_then(|registry| registry.get(skill).and_then(registry_group_token))
        .unwrap_or_else(|| UNGROUPED_CAPABILITY_TOKEN.to_string())
}

fn planner_capability_hint(mapping: &PlannerCapabilityMapping) -> String {
    let mut parts = Vec::new();
    if let Some(action) = mapping.action.as_deref() {
        parts.push(format!("action={action}"));
    }
    if let Some(effect) = mapping.effect {
        parts.push(format!("effect={}", effect.as_token()));
    }
    if !mapping.required.is_empty() {
        parts.push(format!("required={}", mapping.required.join("|")));
    }
    if !mapping.optional.is_empty() {
        parts.push(format!("optional={}", mapping.optional.join("|")));
    }
    if let Some(risk_level) = mapping.risk_level {
        parts.push(format!("risk={}", risk_level_token(risk_level)));
    }
    if mapping.preferred {
        parts.push("preferred=true".to_string());
    }
    if let Some(once_per_task) = mapping.once_per_task {
        parts.push(format!("once_per_task={once_per_task}"));
    }
    if let Some(dedup_scope) = mapping.dedup_scope {
        parts.push(format!("dedup_scope={}", dedup_scope.as_token()));
    }
    if let Some(idempotent) = mapping.idempotent {
        parts.push(format!("idempotent={idempotent}"));
    }
    if let Some(execution_mode) = mapping.execution_mode {
        parts.push(format!("execution_mode={}", execution_mode.as_token()));
    }
    if let Some(async_adapter_kind) = mapping.async_adapter_kind.as_deref() {
        parts.push(format!("async_adapter_kind={async_adapter_kind}"));
    }
    if let Some(isolation_profile) = mapping.isolation_profile {
        parts.push(format!(
            "isolation_profile={}",
            isolation_profile.as_token()
        ));
    }
    if let Some(network_access) = mapping.network_access {
        parts.push(format!("network_access={network_access}"));
    }
    if let Some(filesystem_write) = mapping.filesystem_write {
        parts.push(format!("filesystem_write={filesystem_write}"));
    }
    if let Some(external_publish) = mapping.external_publish {
        parts.push(format!("external_publish={external_publish}"));
    }
    if let Some(credential_access) = mapping.credential_access {
        parts.push(format!("credential_access={credential_access}"));
    }
    if let Some(subprocess) = mapping.subprocess {
        parts.push(format!("subprocess={subprocess}"));
    }
    if let Some(package_install) = mapping.package_install {
        parts.push(format!("package_install={package_install}"));
    }
    if let Some(privilege_escalation) = mapping.privilege_escalation {
        parts.push(format!("privilege_escalation={privilege_escalation}"));
    }
    if let Some(final_answer_shape) = mapping.final_answer_shape.as_deref() {
        parts.push(format!("final_answer_shape={final_answer_shape}"));
    }
    if parts.is_empty() {
        mapping.name.clone()
    } else {
        format!("{}({})", mapping.name, parts.join(","))
    }
}

fn risk_level_token(risk_level: claw_core::skill_registry::SkillRiskLevel) -> &'static str {
    match risk_level {
        claw_core::skill_registry::SkillRiskLevel::Unknown => "unknown",
        claw_core::skill_registry::SkillRiskLevel::Low => "low",
        claw_core::skill_registry::SkillRiskLevel::Medium => "medium",
        claw_core::skill_registry::SkillRiskLevel::High => "high",
    }
}

fn skill_permission_profile_hint(entry: &SkillRegistryEntry) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(risk_level) = entry.risk_level {
        parts.push(format!("risk={}", risk_level_token(risk_level)));
    }
    if let Some(requires_confirmation) = entry.requires_confirmation {
        parts.push(format!("requires_confirmation={requires_confirmation}"));
    }
    if let Some(side_effect) = entry.side_effect {
        parts.push(format!("side_effect={side_effect}"));
    }
    if let Some(auto_invocable) = entry.auto_invocable {
        parts.push(format!("auto_invocable={auto_invocable}"));
    }
    if let Some(once_per_task) = entry.once_per_task {
        parts.push(format!("once_per_task={once_per_task}"));
    }
    if let Some(dedup_scope) = entry.dedup_scope {
        parts.push(format!("dedup_scope={}", dedup_scope.as_token()));
    }
    if let Some(idempotent) = entry.idempotent {
        parts.push(format!("idempotent={idempotent}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

#[cfg(test)]
#[path = "capability_map_tests.rs"]
mod tests;

pub(crate) fn build_compact_capability_map_for_task(
    state: &AppState,
    task: &ClaimedTask,
) -> String {
    build_capability_map_for_task_with_detail(state, task, false)
}

fn build_capability_map_for_task_with_detail(
    state: &AppState,
    task: &ClaimedTask,
    include_registry_skill_hints: bool,
) -> String {
    let execution_policy = crate::task_execution_policy::effective_policy_for_task(state, task);
    let sandbox_diagnostics = crate::process_sandbox::sandbox_backend_diagnostics(
        state.skill_rt.tools_policy.sandbox_backend,
        execution_policy.sandbox_mode,
        crate::process_sandbox::ProcessNetworkPolicy::Deny,
    );
    let sandbox_hint = format!(
        "sandbox_runtime_v1={}",
        serde_json::to_string(&sandbox_diagnostics).unwrap_or_else(|_| {
            "{\"reason_code\":\"sandbox_diagnostics_encode_failed\"}".to_string()
        })
    );
    let execution_policy_hint = format!(
        "task_execution_policy_v1={}",
        execution_policy.to_machine_json()
    );
    let all_visible = state.planner_visible_skills_for_task(task);
    let visible = state.planner_available_skills_for_task(task);
    let available_set = visible.iter().cloned().collect::<BTreeSet<_>>();
    let unavailable_hints = unavailable_skill_hints(state, &all_visible, &available_set);
    let mcp_tools = state.mcp_planner_tools();
    if visible.is_empty() && mcp_tools.is_empty() {
        let mut lines = vec![
            "Current runtime-available tool capabilities are unavailable; use chat only when no external retrieval or execution is needed.".to_string(),
            sandbox_hint,
            execution_policy_hint,
        ];
        if !unavailable_hints.is_empty() {
            lines.push("Enabled but unavailable capabilities omitted from planning:".to_string());
            lines.extend(unavailable_hints);
        }
        return lines.join("\n");
    }

    let mut grouped: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut layered: BTreeMap<PlannerCapabilityKind, BTreeSet<String>> = BTreeMap::new();
    for skill in &visible {
        grouped
            .entry(classify_skill(state, skill))
            .or_default()
            .insert(skill.clone());
        let planner_kind = state
            .get_skills_registry()
            .and_then(|registry| registry.planner_kind(skill))
            .unwrap_or(PlannerCapabilityKind::Skill);
        layered
            .entry(planner_kind)
            .or_default()
            .insert(skill.clone());
    }

    let mut lines = vec![
        "Current capability map (derived from the currently enabled skills):".to_string(),
        "Use this as routing guidance, not as a full tool schema.".to_string(),
        "Do not plan or call capabilities marked `runtime_availability: unavailable`; choose another available capability or explain the dependency gap.".to_string(),
        crate::agent_runtime_contract::runtime_protocol_hint_line(
            &crate::agent_runtime_contract::load_subagent_role_definitions(
                &state
                    .skill_rt
                    .workspace_root
                    .join("configs/agent_guard.toml"),
            ),
        ),
        crate::async_job_contract::async_job_protocol_hint_line(),
        sandbox_hint,
        execution_policy_hint,
    ];

    if !layered.is_empty() {
        lines.push(
            "Capability layers: tools are low-level reusable actions, skills are domain capabilities, workflows are multi-step playbooks."
                .to_string(),
        );
        for (kind, skills) in layered {
            let label = match kind {
                PlannerCapabilityKind::Tool => "tools",
                PlannerCapabilityKind::Skill => "skills",
                PlannerCapabilityKind::Workflow => "workflows",
            };
            lines.push(format!(
                "- {label}: {}.",
                skills.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }

    for (group, skills) in grouped {
        let skills_text = skills.into_iter().collect::<Vec<_>>().join(", ");
        lines.push(format!("- group={group}; visible_skills={skills_text}."));
    }

    if include_registry_skill_hints {
        if let Some(registry) = state.get_skills_registry() {
            let mut hints = Vec::new();
            for skill in &visible {
                let Some(entry) = registry.get(skill) else {
                    continue;
                };
                let aliases = entry
                    .aliases
                    .iter()
                    .map(|alias| alias.trim())
                    .filter(|alias| !alias.is_empty())
                    .take(6)
                    .collect::<Vec<_>>();
                let description = entry
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|description| !description.is_empty());
                let semantic_tags = entry
                    .semantic_tags
                    .iter()
                    .map(|tag| tag.trim())
                    .filter(|tag| !tag.is_empty())
                    .take(8)
                    .collect::<Vec<_>>();
                let validation_actions = entry
                    .validation_actions
                    .iter()
                    .map(|action| action.trim())
                    .filter(|action| !action.is_empty())
                    .take(6)
                    .collect::<Vec<_>>();
                let capability_tokens = entry
                    .resolved_capabilities
                    .iter()
                    .map(|capability| capability.as_token())
                    .take(8)
                    .collect::<Vec<_>>();
                let planner_capability_tokens = entry
                    .planner_capabilities
                    .iter()
                    .map(planner_capability_hint)
                    .take(12)
                    .collect::<Vec<_>>();
                let supported_os = entry
                    .supported_os
                    .iter()
                    .map(|os| os.trim())
                    .filter(|os| !os.is_empty())
                    .take(6)
                    .collect::<Vec<_>>();
                let required_bins = entry
                    .required_bins
                    .iter()
                    .map(|bin| bin.trim())
                    .filter(|bin| !bin.is_empty())
                    .take(8)
                    .collect::<Vec<_>>();
                let optional_bins = entry
                    .optional_bins
                    .iter()
                    .map(|bin| bin.trim())
                    .filter(|bin| !bin.is_empty())
                    .take(8)
                    .collect::<Vec<_>>();
                let platform_notes = entry
                    .platform_notes
                    .iter()
                    .map(|note| note.trim())
                    .filter(|note| !note.is_empty())
                    .take(2)
                    .collect::<Vec<_>>();
                let planner_kind = registry
                    .planner_kind(skill)
                    .unwrap_or(PlannerCapabilityKind::Skill);
                if aliases.is_empty()
                    && description.is_none()
                    && semantic_tags.is_empty()
                    && validation_actions.is_empty()
                    && planner_capability_tokens.is_empty()
                    && capability_tokens.is_empty()
                    && supported_os.is_empty()
                    && required_bins.is_empty()
                    && optional_bins.is_empty()
                    && platform_notes.is_empty()
                    && entry.retryable.is_none()
                    && entry.requires_confirmation.is_none()
                    && !entry.preferred_over_run_cmd
                    && planner_kind == PlannerCapabilityKind::Skill
                {
                    continue;
                }
                let mut parts = Vec::new();
                parts.push(format!("planner_kind: {}", planner_kind.as_token()));
                if let Some(description) = description {
                    parts.push(description.to_string());
                }
                if !semantic_tags.is_empty() {
                    parts.push(format!("semantic_tags: {}", semantic_tags.join(", ")));
                }
                if entry.preferred_over_run_cmd {
                    parts.push("prefer over run_cmd when semantics match".to_string());
                }
                if let Some(permission_profile) = skill_permission_profile_hint(entry) {
                    parts.push(format!("permission_profile={permission_profile}"));
                }
                if !validation_actions.is_empty() {
                    parts.push(format!(
                        "validation_actions: {}",
                        validation_actions.join(", ")
                    ));
                }
                if !planner_capability_tokens.is_empty() {
                    parts.push(format!(
                        "planner_capabilities: {}",
                        planner_capability_tokens.join("; ")
                    ));
                }
                if let Some(retryable) = entry.retryable {
                    parts.push(format!("retryable: {retryable}"));
                }
                if let Some(requires_confirmation) = entry.requires_confirmation {
                    parts.push(format!("requires_confirmation: {requires_confirmation}"));
                }
                if !entry.confirmation_exempt_when.is_empty() {
                    let exemptions = entry
                        .confirmation_exempt_when
                        .iter()
                        .take(4)
                        .map(|matcher| {
                            matcher
                                .iter()
                                .map(|(key, value)| {
                                    format!("{key}={}", compact_toml_value_token(value))
                                })
                                .collect::<Vec<_>>()
                                .join("+")
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    parts.push(format!("confirmation_exempt_when: {exemptions}"));
                }
                parts.extend(skill_availability::availability_metadata_parts(
                    &skill_availability::evaluate_entry_availability(entry),
                ));
                if !capability_tokens.is_empty() {
                    parts.push(format!("capabilities: {}", capability_tokens.join(", ")));
                }
                if !supported_os.is_empty() {
                    parts.push(format!("supported_os: {}", supported_os.join(", ")));
                }
                if !required_bins.is_empty() {
                    parts.push(format!("required_bins: {}", required_bins.join(", ")));
                }
                if !optional_bins.is_empty() {
                    parts.push(format!("optional_bins: {}", optional_bins.join(", ")));
                }
                if !platform_notes.is_empty() {
                    parts.push(format!("platform_notes: {}", platform_notes.join(" | ")));
                }
                if !aliases.is_empty() {
                    parts.push(format!("aliases: {}", aliases.join(", ")));
                }
                hints.push(format!("  - {skill}: {}", parts.join("; ")));
            }
            if !hints.is_empty() {
                lines.push("Registry skill hints:".to_string());
                lines.extend(hints);
            }
        }
    }

    if !mcp_tools.is_empty() {
        lines.push("mcp_tools:".to_string());
        for tool in &mcp_tools {
            let description = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .replace(['\n', '\r'], " ")
                .chars()
                .take(240)
                .collect::<String>();
            let mut fields = vec![
                format!("server={}", tool.server_id),
                format!("effect={}", tool.policy.effect),
                format!("risk={}", tool.policy.risk_level),
                format!("idempotent={}", tool.policy.idempotent),
            ];
            if !tool.required_args.is_empty() {
                fields.push(format!("required={}", tool.required_args.join("|")));
            }
            if !tool.optional_args.is_empty() {
                fields.push(format!("optional={}", tool.optional_args.join("|")));
            }
            if !description.is_empty() {
                fields.push(format!("description={description}"));
            }
            lines.push(format!("  - {}: {}", tool.capability, fields.join(",")));
        }
    }

    if !unavailable_hints.is_empty() {
        lines.push("Enabled but unavailable capabilities omitted from planning:".to_string());
        lines.extend(unavailable_hints);
    }

    lines.push(
        "If the user is asking for current data, real system state, latest external information, or observable results, prefer `act` over `chat`."
            .to_string(),
    );
    lines.push(
        "Use `chat` only for explanation, advice, rewriting, or discussion that does not require external retrieval or execution."
            .to_string(),
    );

    lines.join("\n")
}

fn compact_toml_value_token(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Boolean(v) => v.to_string(),
        toml::Value::Integer(v) => v.to_string(),
        toml::Value::Float(v) => v.to_string(),
        toml::Value::Array(values) => values
            .iter()
            .map(compact_toml_value_token)
            .collect::<Vec<_>>()
            .join("|"),
        _ => value.to_string(),
    }
}

fn unavailable_skill_hints(
    state: &AppState,
    all_visible: &[String],
    available_set: &BTreeSet<String>,
) -> Vec<String> {
    let Some(registry) = state.get_skills_registry() else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    for skill in all_visible {
        if available_set.contains(skill) {
            continue;
        }
        let Some(entry) = registry.get(skill) else {
            continue;
        };
        let availability = skill_availability::evaluate_entry_availability(entry);
        if availability.is_available() {
            continue;
        }
        hints.push(format!(
            "  - {skill}: {}",
            skill_availability::availability_metadata_parts(&availability).join("; ")
        ));
    }
    hints
}
