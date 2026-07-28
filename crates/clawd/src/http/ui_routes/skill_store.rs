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
    let names = configured.cloned().unwrap_or_else(|| {
        claw_core::config::skill_store_optional_skill_names()
            .iter()
            .map(|name| toml::Value::String((*name).to_string()))
            .collect()
    });
    names
        .iter()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| state.resolve_canonical_skill_name(name))
        .collect()
}

fn render_skill_name_array(names: &BTreeSet<String>) -> String {
    let values = names
        .iter()
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

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
    if state.get_skills_registry().is_some_and(|registry| {
        registry
            .get(skill_name)
            .is_some_and(|entry| entry.kind == SkillKind::External)
    }) {
        return true;
    }
    let is_base_skill = state.skill_is_fixed_on(skill_name);
    let is_media_skill = skill_name.starts_with("image_")
        || skill_name.starts_with("audio_")
        || skill_name.starts_with("video_")
        || skill_name.starts_with("music_");

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

fn update_skill_store_installation(
    state: &AppState,
    skill_name: &str,
    installed: bool,
) -> SkillStoreOperationResult<Value> {
    let (raw, parsed) = read_skill_config_file(state).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::ConfigReadFailed,
            error,
        )
    })?;
    let mut switches = collect_skill_switches(&parsed, state);
    let mut uninstalled = collect_uninstalled_skills(&parsed, state);
    if installed {
        uninstalled.remove(skill_name);
        switches.insert(skill_name.to_string(), true);
    } else {
        uninstalled.insert(skill_name.to_string());
        switches.insert(skill_name.to_string(), false);
    }
    let updated = render_skill_store_config(&raw, &switches, &uninstalled);
    write_runtime_config_file(state, &updated).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
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
    let uninstalled = collect_uninstalled_skills(&parsed, &state);
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
            let configured_installed = !uninstalled.contains(&name);
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
                rustclaw_skill_sdk::SkillRuntimeResolver::new(skill_package_root(&state))
                    .resolve(&name)
                    .ok();
            Some(json!({
                "name": name,
                "description": entry.description,
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
                "install_mode": entry.install_mode,
                "build_adapter": manifest.as_ref().map(|value| value.build.adapter.as_token()),
                "build_network_policy": manifest.as_ref().map(|value| match value.build.network {
                    rustclaw_skill_sdk::BuildNetworkPolicy::Deny => "deny",
                    rustclaw_skill_sdk::BuildNetworkPolicy::ApprovalRequired => "approval_required",
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
