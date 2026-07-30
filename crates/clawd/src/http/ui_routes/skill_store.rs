#[derive(Debug, Deserialize)]
struct SkillStoreMutationRequest {
    skill_name: String,
    #[serde(default)]
    allow_network: Option<bool>,
    #[serde(default)]
    preserve_config: Option<bool>,
    #[serde(default)]
    preserve_data: Option<bool>,
}

fn collect_uninstalled_skills(value: &toml::Value, state: &AppState) -> BTreeSet<String> {
    let configured = value
        .get("skills")
        .and_then(|skills| skills.get("uninstalled_skills"))
        .and_then(toml::Value::as_array);
    if let Some(names) = configured {
        return names
            .iter()
            .filter_map(toml::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| state.resolve_canonical_skill_name(name))
            .collect();
    }
    state
        .get_skills_registry()
        .map(|registry| registry.on_demand_names().into_iter().collect())
        .unwrap_or_default()
}

#[cfg(test)]
fn render_skill_name_array(names: &BTreeSet<String>) -> String {
    let values = names
        .iter()
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

#[cfg(test)]
fn render_skill_store_config(
    raw: &str,
    switches: &BTreeMap<String, bool>,
    uninstalled: &BTreeSet<String>,
) -> String {
    let rendered_switches = render_switches_inline_table(switches);
    let updated = upsert_skill_switches_line(raw, &rendered_switches);
    upsert_section_key_line(
        &updated,
        "skills",
        "uninstalled_skills",
        &render_skill_name_array(uninstalled),
    )
}

fn skill_store_item_is_locked(state: &AppState, skill_name: &str) -> bool {
    state.skill_is_fixed_on(skill_name)
        || state
            .get_skills_registry()
            .and_then(|registry| registry.planner_kind(skill_name))
            .is_some_and(|kind| kind == PlannerCapabilityKind::Tool)
}

fn skill_store_item_belongs_to_other_group(state: &AppState, skill_name: &str) -> bool {
    let registry = state.get_skills_registry();
    let entry = registry
        .as_ref()
        .and_then(|registry| registry.get(skill_name));
    if entry.is_some_and(|entry| entry.kind == SkillKind::External) {
        return true;
    }
    if entry.is_some_and(|entry| entry.install_mode.as_deref() == Some("on_demand")) {
        return !skill_store_item_is_locked(state, skill_name);
    }
    let is_base_skill = state.skill_is_fixed_on(skill_name);
    let is_media_skill = entry
        .and_then(|entry| entry.group.as_deref())
        .map(str::trim)
        .is_some_and(|group| matches!(group, "image" | "audio" | "video" | "music"));

    !skill_store_item_is_locked(state, skill_name) && !is_base_skill && !is_media_skill
}

fn validate_skill_store_mutation(
    state: &AppState,
    raw_name: &str,
) -> SkillStoreOperationResult<String> {
    let skill_name = state.resolve_canonical_skill_name(raw_name.trim());
    if skill_name.is_empty() {
        return Err(SkillStoreOperationError::new(
            StatusCode::BAD_REQUEST,
            SkillStoreErrorCode::NameRequired,
            "skill_name=empty",
        ));
    }
    let exists = state
        .get_skills_registry()
        .as_ref()
        .is_some_and(|registry| registry.get(&skill_name).is_some());
    if !exists || hide_skill_in_ui(state, &skill_name) {
        return Err(SkillStoreOperationError::new(
            StatusCode::NOT_FOUND,
            SkillStoreErrorCode::UnknownSkill,
            format!("skill={skill_name}"),
        ));
    }
    if skill_store_item_is_locked(state, &skill_name) {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::LockedSkill,
            format!("skill={skill_name}"),
        ));
    }
    Ok(skill_name)
}

fn bundled_host_policy_grant(
    state: &AppState,
    skill_name: &str,
    manifest: &skill_sdk::PackageManifest,
) -> Result<skill_sdk::HostPolicyGrant, String> {
    let registry = state
        .get_skills_registry()
        .ok_or_else(|| "skills registry is unavailable".to_string())?;
    let entry = registry
        .get(skill_name)
        .ok_or_else(|| format!("registry entry is missing: {skill_name}"))?;
    bundled_host_policy_grant_for_entry(entry, skill_name, manifest)
}

fn bundled_host_policy_grant_for_entry(
    entry: &claw_core::skill_registry::SkillRegistryEntry,
    skill_name: &str,
    manifest: &skill_sdk::PackageManifest,
) -> Result<skill_sdk::HostPolicyGrant, String> {
    use claw_core::skill_registry::{Capability, SkillRiskLevel};
    use skill_sdk::{
        ApprovalSource, GrantedCapability, HostPolicyGrant, HostRiskLevel,
        HOST_POLICY_GRANT_SCHEMA_VERSION,
    };

    let request = manifest
        .effective_capability_request()
        .map_err(|error| error.to_string())?;
    let planner_grants = |selector: fn(&claw_core::skill_registry::PlannerCapabilityMapping) -> bool| {
        entry.planner_capabilities.iter().any(selector)
    };
    let permission_checks = [
        (
            request.permissions.llm_gateway,
            entry.resolved_capabilities.contains(&Capability::Llm),
            "llm_gateway",
        ),
        (
            request.permissions.network,
            entry.resolved_capabilities.contains(&Capability::Net)
                || planner_grants(|mapping| mapping.network_access == Some(true)),
            "network",
        ),
        (
            request.permissions.filesystem_read,
            entry.resolved_capabilities.contains(&Capability::FsRead),
            "filesystem_read",
        ),
        (
            request.permissions.filesystem_write,
            entry.resolved_capabilities.contains(&Capability::FsWrite)
                || planner_grants(|mapping| mapping.filesystem_write == Some(true)),
            "filesystem_write",
        ),
        (
            request.permissions.subprocess,
            entry.resolved_capabilities.contains(&Capability::Exec)
                || planner_grants(|mapping| mapping.subprocess == Some(true)),
            "subprocess",
        ),
        (
            request.permissions.package_install,
            entry.resolved_capabilities.contains(&Capability::Exec)
                || planner_grants(|mapping| mapping.package_install == Some(true)),
            "package_install",
        ),
        (
            request.permissions.privilege_escalation,
            entry.resolved_capabilities.contains(&Capability::ExecSudo)
                || planner_grants(|mapping| mapping.privilege_escalation == Some(true)),
            "privilege_escalation",
        ),
        (
            request.permissions.external_publish,
            planner_grants(|mapping| mapping.external_publish == Some(true)),
            "external_publish",
        ),
    ];
    for (requested, granted, permission) in permission_checks {
        if requested && !granted {
            return Err(format!(
                "manifest requests {permission} but bundled registry does not grant it"
            ));
        }
    }
    for credential in &request.permissions.credential_refs {
        if !entry
            .resolved_capabilities
            .contains(&Capability::Secrets(credential.clone()))
        {
            return Err(format!(
                "manifest requests ungranted credential reference: {credential}"
            ));
        }
    }
    let risk_level = match entry.risk_level.unwrap_or(SkillRiskLevel::High) {
        SkillRiskLevel::Low => HostRiskLevel::Low,
        SkillRiskLevel::Medium => HostRiskLevel::Medium,
        SkillRiskLevel::High | SkillRiskLevel::Unknown => HostRiskLevel::High,
    };
    let grant = HostPolicyGrant {
        schema_version: HOST_POLICY_GRANT_SCHEMA_VERSION,
        skill_name: skill_name.to_string(),
        version: manifest.package.version.clone(),
        semantic_contract_digest: manifest
            .capability_request_digest()
            .map_err(|error| error.to_string())?,
        capabilities: request
            .capabilities
            .iter()
            .map(|capability| GrantedCapability {
                name: capability.name.clone(),
                action: capability.action.clone(),
            })
            .collect(),
        permissions: request.permissions,
        risk_level,
        auto_invocable: entry.auto_invocable.unwrap_or(false),
        requires_confirmation: entry
            .requires_confirmation
            .unwrap_or(risk_level == HostRiskLevel::High),
        approval_source: ApprovalSource::ReleaseBaseline,
        approved_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .max(1),
    };
    grant
        .validate_against(manifest)
        .map_err(|error| error.to_string())?;
    Ok(grant)
}

pub(crate) fn repair_bundled_skill_admission_offline(
    workspace_root: &Path,
    config: &claw_core::config::AppConfig,
    skill_name: &str,
) -> Result<crate::skill_admission::OverlaySnapshot, String> {
    let skill_name = skill_name.trim();
    if skill_name.is_empty() {
        return Err("skill name is required".to_string());
    }
    let registry_path = config
        .skills
        .registry_path
        .as_deref()
        .ok_or_else(|| "skills.registry_path is required".to_string())?;
    let registry_path = if Path::new(registry_path).is_absolute() {
        PathBuf::from(registry_path)
    } else {
        workspace_root.join(registry_path)
    };
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)?;
    let entry = registry
        .get(skill_name)
        .ok_or_else(|| format!("bundled registry entry is missing: {skill_name}"))?;
    if entry.install_mode.as_deref() != Some("on_demand") {
        return Err(format!(
            "offline admission repair requires an on-demand bundled skill: {skill_name}"
        ));
    }
    let verified = skill_sdk::InstallReceiptStore::new(
        workspace_root.join("data/skill-packages"),
    )
    .verified_current_install(skill_name)
    .map_err(|error| format!("current install is not verified: {error}"))?;
    let grant = bundled_host_policy_grant_for_entry(entry, skill_name, &verified.manifest)?;
    let prompt_path = workspace_root.join(&entry.prompt_file);
    let prompt = fs::read_to_string(&prompt_path)
        .map_err(|error| format!("prompt read failed: path={} error={error}", prompt_path.display()))?;
    let manifest_path = registry
        .package_manifest_path(skill_name)
        .ok_or_else(|| format!("package manifest is missing: {skill_name}"))?;
    crate::skill_admission::SkillAdmissionService::from_config(workspace_root, config)
        .map_err(|error| error.to_string())?
        .admit_bundled(crate::skill_admission::AdmissionMutation {
            metadata: crate::skill_admission::ExternalSkillMetadata {
                name: skill_name.to_string(),
                source: crate::skill_admission::SkillAdmissionSource::BundledBase,
                package_manifest_path: manifest_path.to_string(),
                description: entry.description.clone().unwrap_or_default(),
                aliases: entry.aliases.clone(),
                group: entry
                    .group
                    .clone()
                    .unwrap_or_else(|| "extensions".to_string()),
            },
            prompt,
            state: skill_sdk::AdmissionState::Enabled,
            grant: Some(grant),
        })
        .map_err(|error| error.to_string())
}

fn update_skill_store_installation(
    state: &AppState,
    skill_name: &str,
    installed: bool,
) -> SkillStoreOperationResult<Value> {
    let service = admission_service(state).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::ConfigWriteFailed,
            error,
        )
    })?;
    let before = match service.snapshot() {
        Ok(snapshot) => Some(snapshot),
        Err(_snapshot_error)
            if installed
                && service.is_bundled_skill(skill_name).map_err(|error| {
                    SkillStoreOperationError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        SkillStoreErrorCode::ConfigWriteFailed,
                        error,
                    )
                })? =>
        {
            // A repair may be the only way to replace a bundled skill whose
            // previously pinned package was lost. commit_mutation still
            // validates the complete replacement generation before activation.
            None
        }
        Err(error) => {
            return Err(SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::ConfigWriteFailed,
                error,
            ));
        }
    };
    if !installed
        && before
            .as_ref()
            .and_then(|snapshot| snapshot.state(skill_name))
            == Some(skill_sdk::AdmissionState::Tombstoned)
    {
        let reload = reload_skill_views(state).map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::RuntimeReloadFailed,
                error,
            )
        })?;
        return Ok(json!({
            "skill_name": skill_name,
            "installed": false,
            "enabled": false,
            "already_tombstoned": true,
            "reload": reload,
        }));
    }
    let verified = skill_sdk::InstallReceiptStore::new(skill_package_root(state))
        .verified_current_install(skill_name)
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?;
    let grant = bundled_host_policy_grant(state, skill_name, &verified.manifest).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::ConfigWriteFailed,
            error,
        )
    })?;
    let registry = state.get_skills_registry().ok_or_else(|| {
        SkillStoreOperationError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            SkillStoreErrorCode::RegistryUnavailable,
            "registry=unavailable",
        )
    })?;
    let entry = registry.get(skill_name).ok_or_else(|| {
        SkillStoreOperationError::new(
            StatusCode::NOT_FOUND,
            SkillStoreErrorCode::UnknownSkill,
            format!("skill={skill_name}"),
        )
    })?;
    let prompt = crate::load_prompt_template_for_state(state, &entry.prompt_file, "").0;
    let target_state = if installed {
        skill_sdk::AdmissionState::Enabled
    } else {
        skill_sdk::AdmissionState::Tombstoned
    };
    let snapshot_result = if before
        .as_ref()
        .and_then(|snapshot| snapshot.source(skill_name))
        == Some(crate::skill_admission::SkillAdmissionSource::ExternalOverlay)
    {
        service.set_state(skill_name, target_state, None)
    } else {
        service.admit_bundled(crate::skill_admission::AdmissionMutation {
            metadata: crate::skill_admission::ExternalSkillMetadata {
                name: skill_name.to_string(),
                source: crate::skill_admission::SkillAdmissionSource::BundledBase,
                package_manifest_path: registry
                    .package_manifest_path(skill_name)
                    .unwrap_or_default()
                    .to_string(),
                description: entry.description.clone().unwrap_or_default(),
                aliases: entry.aliases.clone(),
                group: entry
                    .group
                    .clone()
                    .unwrap_or_else(|| "extensions".to_string()),
            },
            prompt,
            state: target_state,
            grant: Some(grant),
        })
    };
    let snapshot = snapshot_result.map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::ConfigWriteFailed,
                error,
            )
        })?;
    let reload = reload_skill_views(state).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::RuntimeReloadFailed,
            error,
        )
    })?;
    Ok(json!({
        "skill_name": skill_name,
        "installed": installed,
        "enabled": installed,
        "registry_generation": snapshot.generation,
        "registry_generation_digest": snapshot.generation_digest,
        "reload": reload,
    }))
}

async fn get_skill_store_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    if let Err(error) = initialize_skill_store_operations(&state) {
        return skill_store_error_response(error);
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, parsed)) => parsed,
        Err(error) => {
            return skill_store_error_response(SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::ConfigReadFailed,
                error,
            ));
        }
    };
    let mut uninstalled = collect_uninstalled_skills(&parsed, &state);
    let admission_snapshot = admission_service(&state)
        .and_then(|service| service.snapshot().map_err(|error| error.to_string()))
        .ok();
    if let Some(snapshot) = &admission_snapshot {
        for name in snapshot
            .enabled
            .iter()
            .chain(snapshot.disabled.iter())
            .chain(snapshot.awaiting_policy.iter())
        {
            uninstalled.remove(name);
        }
        uninstalled.extend(snapshot.tombstoned.iter().cloned());
    }
    let Some(registry) = state.get_skills_registry() else {
        return skill_store_error_response(SkillStoreOperationError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            SkillStoreErrorCode::RegistryUnavailable,
            "registry=unavailable",
        ));
    };
    let runtime_enabled = state.get_skills_list();
    let operation_store = skill_store_operation_store(&state);
    let active_operation = operation_store.latest_active().ok().flatten();
    let recent_operations = operation_store
        .list()
        .unwrap_or_default()
        .into_iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>();
    let mut names = registry.all_names();
    names.sort_unstable();
    let items = names
        .into_iter()
        .filter(|name| !hide_skill_in_ui(&state, name))
        .filter(|name| skill_store_item_belongs_to_other_group(&state, name))
        .filter_map(|name| {
            let entry = registry.get(&name)?;
            let configured_installed = admission_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.state(&name))
                .map(|admission_state| {
                    admission_state != skill_sdk::AdmissionState::Tombstoned
                })
                .unwrap_or_else(|| !uninstalled.contains(&name));
            let package_available = if matches!(entry.kind, SkillKind::Runner | SkillKind::External)
            {
                skill_store_package_available(&state, &registry, &name)
            } else {
                true
            };
            let installed = configured_installed && package_available;
            let installation_issue = if configured_installed && !package_available {
                Some("package_missing")
            } else {
                None
            };
            let (config_files, existing_config_files) = skill_config_state(&state, &name);
            let storage = registry.storage(&name);
            let private_data_state = storage
                .map(|_| state.core.skill_storage.data_state(&name))
                .transpose()
                .ok()
                .flatten();
            let manifest = skill_store_manifest_metadata(&state, &registry, &name);
            let installed_launch =
                skill_sdk::SkillRuntimeResolver::new(skill_package_root(&state))
                    .resolve(&name)
                    .ok();
            Some(json!({
                "name": name,
                "description": entry.description,
                "description_zh": entry.description_zh,
                "group": entry.group,
                "catalog_section": "other",
                "kind": skill_kind_token(entry.kind),
                "source_kind": if entry.kind == SkillKind::External {
                    "third_party"
                } else if entry.install_mode.as_deref() == Some("on_demand") {
                    "bundled_optional"
                } else {
                    "bundled_core"
                },
                "source": manifest.as_ref().and_then(|value| value.package.source.as_deref()),
                "installed": installed,
                "configured_installed": configured_installed,
                "package_available": package_available,
                "installation_issue": installation_issue,
                "enabled": installed && runtime_enabled.contains(&entry.name),
                "admission_state": admission_snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.state(&name))
                    .and_then(|state| serde_json::to_value(state).ok()),
                "install_mode": entry.install_mode,
                "build_adapter": manifest.as_ref().map(|value| value.build.adapter.as_token()),
                "build_network_policy": manifest.as_ref().map(|value| match value.build.network {
                    skill_sdk::BuildNetworkPolicy::Deny => "deny",
                    skill_sdk::BuildNetworkPolicy::ApprovalRequired => "approval_required",
                }),
                "supported_os": manifest.as_ref().map(|value| &value.package.supported_os),
                "supported_arch": manifest.as_ref().map(|value| &value.package.supported_arch),
                "package_version": manifest.as_ref().map(|value| value.package.version.as_str()),
                "installed_version": installed_launch.as_ref().map(|value| value.version.as_str()),
                "protocol": manifest.as_ref().map(|value| value.package.protocol.as_str()),
                "config_files": config_files,
                "existing_config_files": existing_config_files,
                "storage_kind": storage.map(|value| value.kind.as_str()),
                "private_data_state": private_data_state,
                "skill": build_skill_list_item(&state, &entry.name),
            }))
        })
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "items": items,
                "uninstalled_skill_names": uninstalled,
                "active_operation": active_operation,
                "recent_operations": recent_operations,
            })),
            error: None,
        }),
    )
}
