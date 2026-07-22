#[derive(Debug, Deserialize)]
struct SkillStoreMutationRequest {
    skill_name: String,
    #[serde(default)]
    preserve_config: Option<bool>,
}

fn collect_uninstalled_skills(value: &toml::Value, state: &AppState) -> BTreeSet<String> {
    let configured = value
        .get("skills")
        .and_then(|skills| skills.get("uninstalled_skills"))
        .and_then(toml::Value::as_array);
    let names = configured
        .cloned()
        .unwrap_or_else(|| {
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
    claw_core::config::core_skills_always_enabled()
        .iter()
        .any(|name| state.resolve_canonical_skill_name(name) == skill_name)
        || state
            .get_skills_registry()
            .and_then(|registry| registry.planner_kind(skill_name))
            .is_some_and(|kind| kind == PlannerCapabilityKind::Tool)
}

fn skill_store_item_belongs_to_other_group(state: &AppState, skill_name: &str) -> bool {
    let is_base_skill = claw_core::config::base_skill_names()
        .iter()
        .any(|name| state.resolve_canonical_skill_name(name) == skill_name);
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
    let mut names = registry.all_names();
    names.sort_unstable();
    let items = names
        .into_iter()
        .filter(|name| !hide_skill_in_ui(&state, name))
        .filter(|name| skill_store_item_belongs_to_other_group(&state, name))
        .filter_map(|name| {
            let entry = registry.get(&name)?;
            let configured_installed = !uninstalled.contains(&name);
            let runner_available = if entry.kind == SkillKind::Runner {
                runner_binary_name(&registry.runner_name(&name))
                    .ok()
                    .map(|binary| {
                        state
                            .skill_rt
                            .workspace_root
                            .join("target/release")
                            .join(binary)
                            .is_file()
                    })
                    .unwrap_or(false)
            } else {
                true
            };
            let installed = configured_installed && runner_available;
            let installation_issue = if configured_installed && !runner_available {
                Some("runner_missing")
            } else {
                None
            };
            let (config_files, existing_config_files) = skill_config_state(&state, &name);
            Some(json!({
                "name": name,
                "description": entry.description,
                "group": entry.group,
                "catalog_section": "other",
                "kind": skill_kind_token(entry.kind),
                "source_kind": if entry.kind == SkillKind::External { "third_party" } else { "bundled" },
                "source": entry.external_source_url,
                "installed": installed,
                "configured_installed": configured_installed,
                "runner_available": runner_available,
                "installation_issue": installation_issue,
                "enabled": installed && runtime_enabled.contains(&entry.name),
                "install_mode": entry.install_mode,
                "config_files": config_files,
                "existing_config_files": existing_config_files,
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
            })),
            error: None,
        }),
    )
}

async fn install_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    let skill_name = match validate_skill_store_mutation(&state, &request.skill_name) {
        Ok(name) => name,
        Err(error) => return skill_store_error_response(error),
    };
    let _mutation_permit = match begin_skill_store_mutation(&state) {
        Ok(permit) => permit,
        Err(error) => return skill_store_error_response(error),
    };
    let spec = match skill_store_install_spec(&state, &skill_name) {
        Ok(spec) => spec,
        Err(error) => return skill_store_error_response(error),
    };
    let binary_path = match spec.as_ref() {
        Some(spec) => match compile_skill_store_runner(&state, spec).await {
            Ok(path) => Some(path),
            Err(error) => return skill_store_error_response(error),
        },
        None => None,
    };
    match update_skill_store_installation(&state, &skill_name, true) {
        Ok(mut data) => {
            let (_, existing_config_files) = skill_config_state(&state, &skill_name);
            if let Some(object) = data.as_object_mut() {
                object.insert("compiled".to_string(), json!(spec.is_some()));
                object.insert(
                    "binary_path".to_string(),
                    json!(binary_path.as_ref().and_then(|path| path
                        .strip_prefix(&state.skill_rt.workspace_root)
                        .ok())
                        .map(|path| path.to_string_lossy().into_owned())),
                );
                object.insert(
                    "reused_config_files".to_string(),
                    json!(existing_config_files),
                );
            }
            (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
            )
        }
        Err(error) => skill_store_error_response(error),
    }
}

async fn remove_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    let skill_name = match validate_skill_store_mutation(&state, &request.skill_name) {
        Ok(name) => name,
        Err(error) => return skill_store_error_response(error),
    };
    let _mutation_permit = match begin_skill_store_mutation(&state) {
        Ok(permit) => permit,
        Err(error) => return skill_store_error_response(error),
    };
    let preserve_config = request.preserve_config.unwrap_or(true);
    let spec = match skill_store_install_spec(&state, &skill_name) {
        Ok(spec) => spec,
        Err(error) => return skill_store_error_response(error),
    };
    match update_skill_store_installation(&state, &skill_name, false) {
        Ok(mut data) => {
            let binary_removed = match spec.as_ref() {
                Some(spec) => match remove_skill_store_binary(&state, spec) {
                    Ok(removed) => removed,
                    Err(error) => return skill_store_error_response(error),
                },
                None => false,
            };
            let deleted_config_files = if preserve_config {
                Vec::new()
            } else {
                match delete_declared_skill_configs(&state, &skill_name) {
                    Ok(paths) => paths,
                    Err(error) => return skill_store_error_response(error),
                }
            };
            if let Some(object) = data.as_object_mut() {
                object.insert("binary_removed".to_string(), json!(binary_removed));
                object.insert("config_preserved".to_string(), json!(preserve_config));
                object.insert(
                    "deleted_config_files".to_string(),
                    json!(deleted_config_files),
                );
            }
            (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
            )
        }
        Err(error) => skill_store_error_response(error),
    }
}
