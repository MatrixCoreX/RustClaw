fn normalize_planner_capability_aliases(
    aliases: &BTreeMap<String, String>,
    skill: &str,
    path: &Path,
) -> Result<BTreeMap<String, String>, String> {
    let mut normalized = BTreeMap::new();
    for (raw_alias, raw_target) in aliases {
        let alias = normalize_planner_capability_name(raw_alias);
        let target = normalize_planner_capability_name(raw_target);
        if alias.is_empty() || target.is_empty() {
            return Err(format!(
                "empty planner capability alias for skill `{skill}` in {}",
                path.display()
            ));
        }
        if alias == target {
            return Err(format!(
                "self-referential planner capability alias `{alias}` for skill `{skill}` in {}",
                path.display()
            ));
        }
        if normalized.insert(alias.clone(), target).is_some() {
            return Err(format!(
                "duplicate normalized planner capability alias `{alias}` for skill `{skill}` in {}",
                path.display()
            ));
        }
    }
    Ok(normalized)
}

fn planner_capability_policy_equivalent(
    alias: &PlannerCapabilityMapping,
    target: &PlannerCapabilityMapping,
) -> bool {
    alias.required_companions == target.required_companions
        && alias.action == target.action
        && alias.effect == target.effect
        && alias.risk_level == target.risk_level
        && alias.auto_invocable == target.auto_invocable
        && alias.requires_confirmation == target.requires_confirmation
        && alias.once_per_task == target.once_per_task
        && alias.dedup_scope == target.dedup_scope
        && alias.dedup_fields == target.dedup_fields
        && alias.idempotent == target.idempotent
        && alias.execution_mode == target.execution_mode
        && alias.timeout_seconds == target.timeout_seconds
        && alias.async_adapter_kind == target.async_adapter_kind
        && alias.isolation_profile == target.isolation_profile
        && alias.network_access == target.network_access
        && alias.filesystem_write == target.filesystem_write
        && alias.external_publish == target.external_publish
        && alias.credential_access == target.credential_access
        && alias.subprocess == target.subprocess
        && alias.package_install == target.package_install
        && alias.privilege_escalation == target.privilege_escalation
        && alias.reconciliation_capability == target.reconciliation_capability
        && alias.approval_preview_fields == target.approval_preview_fields
        && alias.final_answer_shape == target.final_answer_shape
}

fn planner_capability_argument_surface_covers(
    alias: &PlannerCapabilityMapping,
    target: &PlannerCapabilityMapping,
) -> bool {
    let target_args = target
        .required
        .iter()
        .chain(target.optional.iter())
        .collect::<std::collections::BTreeSet<_>>();
    alias
        .required
        .iter()
        .chain(alias.optional.iter())
        .all(|arg| target_args.contains(arg))
}

fn validate_planner_capability_aliases(
    entry: &SkillRegistryEntry,
    path: &Path,
) -> Result<(), String> {
    for (alias_name, target_name) in &entry.planner_capability_aliases {
        if entry.planner_capability_aliases.contains_key(target_name) {
            return Err(format!(
                "planner capability alias chain `{alias_name}` -> `{target_name}` is not allowed for skill `{}` in {}",
                entry.name,
                path.display()
            ));
        }
        let alias = entry
            .planner_capabilities
            .iter()
            .find(|mapping| mapping.name == *alias_name)
            .ok_or_else(|| {
                format!(
                    "planner capability alias `{alias_name}` is not a declared mapping for skill `{}` in {}",
                    entry.name,
                    path.display()
                )
            })?;
        let target = entry
            .planner_capabilities
            .iter()
            .find(|mapping| mapping.name == *target_name)
            .ok_or_else(|| {
                format!(
                    "planner capability alias target `{target_name}` is not a declared mapping for skill `{}` in {}",
                    entry.name,
                    path.display()
                )
            })?;
        if !planner_capability_policy_equivalent(alias, target) {
            return Err(format!(
                "planner capability alias policy mismatch `{alias_name}` -> `{target_name}` for skill `{}` in {}",
                entry.name,
                path.display()
            ));
        }
        if !planner_capability_argument_surface_covers(alias, target) {
            return Err(format!(
                "planner capability alias arguments are not covered by `{target_name}` for alias `{alias_name}` in skill `{}` ({})",
                entry.name,
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_planner_capability_schemas(
    entry: &SkillRegistryEntry,
    path: &Path,
) -> Result<(), String> {
    let input_schema = entry.input_schema.as_ref().and_then(toml_value_to_json);
    for mapping in &entry.planner_capabilities {
        if mapping
            .timeout_seconds
            .is_some_and(|timeout| !(1..=86_400).contains(&timeout))
        {
            return Err(format!(
                "planner capability timeout must be within 1..=86400 seconds for skill `{}` capability `{}` in {}",
                entry.name,
                mapping.name,
                path.display()
            ));
        }
        planner_capability_argument_schema(input_schema.as_ref(), mapping)
            .map_err(|error| format!("{error} for skill `{}` in {}", entry.name, path.display()))?;
        validate_approval_preview_fields(entry, mapping, path)?;
    }
    Ok(())
}
