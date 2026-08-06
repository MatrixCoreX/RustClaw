pub(super) fn validate_named_capability(
    name: &str,
    token: &str,
    label: &str,
) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!("{label} name length must be 1..=64: `{token}`"));
    }
    if !name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    }) {
        return Err(format!("{label} name must match [a-z0-9_]: `{token}`"));
    }
    Ok(())
}

pub(super) fn validate_required_companion_capabilities(
    registry: &SkillsRegistry,
    path: &Path,
) -> Result<(), String> {
    let declared = registry
        .by_name
        .values()
        .flat_map(|entry| {
            entry
                .planner_capabilities
                .iter()
                .filter(|mapping| !entry.planner_capability_aliases.contains_key(&mapping.name))
        })
        .map(|mapping| mapping.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for (skill_name, entry) in &registry.by_name {
        for mapping in &entry.planner_capabilities {
            for companion in &mapping.required_companions {
                if companion == &mapping.name {
                    return Err([
                        "planner_required_companion_self".to_string(),
                        mapping.name.clone(),
                        skill_name.clone(),
                        path.display().to_string(),
                    ]
                    .join(":"));
                }
                if !declared.contains(companion.as_str()) {
                    return Err([
                        "planner_required_companion_unknown".to_string(),
                        mapping.name.clone(),
                        skill_name.clone(),
                        companion.clone(),
                        path.display().to_string(),
                    ]
                    .join(":"));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_global_planner_capability_aliases(
    registry: &SkillsRegistry,
    path: &Path,
) -> Result<(), String> {
    let mut aliases = BTreeMap::<String, (String, String)>::new();
    for (skill_name, entry) in &registry.by_name {
        for (alias, target) in &entry.planner_capability_aliases {
            if let Some((existing_skill, existing_target)) =
                aliases.insert(alias.clone(), (skill_name.clone(), target.clone()))
            {
                return Err([
                    "planner_capability_alias_duplicate",
                    alias,
                    &existing_skill,
                    &existing_target,
                    skill_name,
                    target,
                    &path.display().to_string(),
                ]
                .join(":"));
            }
        }
    }
    for (alias, (skill_name, target)) in &aliases {
        if let Some((target_skill, next_target)) = aliases.get(target) {
            return Err([
                "planner_capability_alias_chain",
                alias,
                skill_name,
                target,
                target_skill,
                next_target,
                &path.display().to_string(),
            ]
            .join(":"));
        }
    }
    Ok(())
}

pub(super) fn validate_package_version(
    skill_name: &str,
    version: Option<&str>,
    path: &Path,
) -> Result<(), String> {
    if version.is_some_and(|version| {
        version.len() > 64
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    }) {
        return Err([
            "skill_package_version_invalid".to_string(),
            skill_name.to_string(),
            path.display().to_string(),
        ]
        .join(":"));
    }
    Ok(())
}

pub(super) fn validate_approval_preview_fields(
    entry: &SkillRegistryEntry,
    mapping: &PlannerCapabilityMapping,
    path: &Path,
) -> Result<(), String> {
    if mapping.approval_preview_fields.len() > 16 {
        return Err(format!(
            "approval preview field limit exceeded for skill `{}` capability `{}` in {}",
            entry.name,
            mapping.name,
            path.display()
        ));
    }
    let mut declared = mapping.optional.iter().cloned().collect::<BTreeSet<_>>();
    for requirement in &mapping.required {
        for alternative in planner_requirement_alternatives(requirement) {
            declared.extend(alternative);
        }
    }
    for field in &mapping.approval_preview_fields {
        let sensitive = [
            "secret",
            "password",
            "api_key",
            "credential",
            "access_token",
        ]
        .iter()
        .any(|token| field.contains(token));
        if sensitive || !declared.contains(field) {
            return Err([
                "planner_approval_preview_invalid".to_string(),
                field.clone(),
                entry.name.clone(),
                mapping.name.clone(),
                path.display().to_string(),
            ]
            .join(":"));
        }
    }
    Ok(())
}

pub(super) fn validate_reconciliation_capabilities(
    registry: &SkillsRegistry,
    path: &Path,
) -> Result<(), String> {
    let mappings = registry
        .by_name
        .values()
        .flat_map(|entry| entry.planner_capabilities.iter())
        .collect::<Vec<_>>();
    for source in &mappings {
        let Some(target_name) = source.reconciliation_capability.as_deref() else {
            continue;
        };
        if source.effect != Some(PlannerCapabilityEffect::External)
            || source.idempotent != Some(false)
        {
            return Err([
                "planner_reconciliation_source_invalid".to_string(),
                target_name.to_string(),
                source.name.clone(),
                path.display().to_string(),
            ]
            .join(":"));
        }
        let targets = mappings
            .iter()
            .filter(|mapping| mapping.name == target_name)
            .collect::<Vec<_>>();
        if targets.len() != 1
            || targets[0].effect != Some(PlannerCapabilityEffect::Observe)
            || targets[0].external_publish != Some(false)
        {
            return Err(format!(
                "reconciliation capability `{target_name}` for `{}` must resolve uniquely to an observe capability with external_publish=false in {}",
                source.name,
                path.display()
            ));
        }
    }
    Ok(())
}

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{
    planner_requirement_alternatives, PlannerCapabilityEffect, PlannerCapabilityMapping,
    SkillRegistryEntry, SkillsRegistry,
};
