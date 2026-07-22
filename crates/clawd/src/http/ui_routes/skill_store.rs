#[derive(Debug, Deserialize)]
struct SkillStoreMutationRequest {
    skill_name: String,
}

fn collect_uninstalled_skills(value: &toml::Value, state: &AppState) -> BTreeSet<String> {
    value
        .get("skills")
        .and_then(|skills| skills.get("uninstalled_skills"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
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
) -> Result<String, (StatusCode, Json<ApiResponse<Value>>)> {
    let skill_name = state.resolve_canonical_skill_name(raw_name.trim());
    if skill_name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skill_name is required".to_string()),
            }),
        ));
    }
    let exists = state
        .get_skills_registry()
        .as_ref()
        .is_some_and(|registry| registry.get(&skill_name).is_some());
    if !exists || hide_skill_in_ui(state, &skill_name) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unknown skill: {skill_name}")),
            }),
        ));
    }
    if skill_store_item_is_locked(state, &skill_name) {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("locked skills cannot be removed from the runtime".to_string()),
            }),
        ));
    }
    Ok(skill_name)
}

fn update_skill_store_installation(
    state: &AppState,
    skill_name: &str,
    installed: bool,
) -> Result<Value, String> {
    let (raw, parsed) = read_skill_config_file(state)
        .map_err(|error| format!("read skills config failed: {error}"))?;
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
    write_runtime_config_file(state, &updated)
        .map_err(|error| format!("write skills config failed: {error}"))?;
    let reload = reload_skill_views(state)
        .map_err(|error| format!("reload skill views failed: {error}"))?;
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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {error}")),
                }),
            );
        }
    };
    let uninstalled = collect_uninstalled_skills(&parsed, &state);
    let Some(registry) = state.get_skills_registry() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skills registry is not available".to_string()),
            }),
        );
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
            let installed = !uninstalled.contains(&name);
            Some(json!({
                "name": name,
                "description": entry.description,
                "group": entry.group,
                "catalog_section": "other",
                "kind": skill_kind_token(entry.kind),
                "source_kind": if entry.kind == SkillKind::External { "third_party" } else { "bundled" },
                "source": entry.external_source_url,
                "installed": installed,
                "enabled": installed && runtime_enabled.contains(&entry.name),
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
        Err(response) => return response,
    };
    match update_skill_store_installation(&state, &skill_name, true) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(error),
            }),
        ),
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
        Err(response) => return response,
    };
    match update_skill_store_installation(&state, &skill_name, false) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(error),
            }),
        ),
    }
}
