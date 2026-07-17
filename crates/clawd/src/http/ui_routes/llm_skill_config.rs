fn llm_vendor_names() -> [&'static str; 9] {
    [
        "openai",
        "google",
        "anthropic",
        "grok",
        "deepseek",
        "qwen",
        "minimax",
        "mimo",
        "custom",
    ]
}

fn llm_vendor_supports_api_format(vendor_name: &str) -> bool {
    matches!(
        vendor_name.trim().to_ascii_lowercase().as_str(),
        "minimax" | "mimo"
    )
}

fn collect_llm_vendor_info(value: &toml::Value) -> Vec<Value> {
    let mut vendors = Vec::new();
    let Some(llm) = value.get("llm").and_then(|v| v.as_table()) else {
        return vendors;
    };
    for vendor_name in llm_vendor_names() {
        let Some(vendor) = llm.get(vendor_name).and_then(|v| v.as_table()) else {
            continue;
        };
        let base_url = vendor
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let default_model = vendor
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let api_key_configured = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let api_key_masked = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(mask_secret);
        let api_key = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let api_format = if llm_vendor_supports_api_format(vendor_name) {
            normalize_llm_api_format(vendor.get("api_format").and_then(|v| v.as_str()))
        } else {
            String::new()
        };
        let mut models = vendor
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !default_model.is_empty() && !models.iter().any(|m| m == &default_model) {
            models.insert(0, default_model.clone());
        }
        vendors.push(json!({
            "name": vendor_name,
            "default_model": default_model,
            "models": models,
            "base_url": base_url,
            "api_format": api_format,
            "api_key": api_key,
            "api_key_configured": api_key_configured,
            "api_key_masked": api_key_masked
        }));
    }
    vendors
}

fn normalize_llm_api_format(raw: Option<&str>) -> String {
    let fmt = raw.unwrap_or("").trim();
    if fmt.eq_ignore_ascii_case("anthropic") || fmt.eq_ignore_ascii_case("anthropic_claude") {
        "anthropic_claude".to_string()
    } else {
        "openai_compat".to_string()
    }
}

fn current_runtime_llm_info(state: &AppState) -> Value {
    if let Some(provider) = state.core.llm_providers.first() {
        let vendor = provider
            .config
            .name
            .strip_prefix("vendor-")
            .unwrap_or(provider.config.name.as_str())
            .to_string();
        return json!({
            "vendor": vendor,
            "model": provider.config.model,
            "provider_name": provider.config.name,
            "provider_type": provider.config.provider_type
        });
    }
    json!(null)
}

fn saved_llm_vendor_runtime_fields(
    parsed: &toml::Value,
    selected_vendor: &str,
) -> (String, String, String) {
    let section_key = format!("llm.{selected_vendor}");
    let vendor = parsed
        .get("llm")
        .and_then(|llm| llm.get(selected_vendor))
        .or_else(|| parsed.get(&section_key));
    let base_url = vendor
        .and_then(|v| v.get("base_url"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let api_key = vendor
        .and_then(|v| v.get("api_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let provider_type = if llm_vendor_supports_api_format(selected_vendor) {
        normalize_llm_api_format(
            vendor
                .and_then(|v| v.get("api_format"))
                .and_then(|v| v.as_str()),
        )
    } else {
        String::new()
    };
    (base_url, api_key, provider_type)
}

fn llm_provider_type_for_vendor(selected_vendor: &str, vendor_api_format: Option<&str>) -> String {
    if llm_vendor_supports_api_format(selected_vendor) {
        normalize_llm_api_format(vendor_api_format)
    } else if selected_vendor.trim().eq_ignore_ascii_case("google") {
        "google_gemini".to_string()
    } else if selected_vendor.trim().eq_ignore_ascii_case("anthropic") {
        "anthropic_claude".to_string()
    } else {
        "openai_compat".to_string()
    }
}

fn build_llm_test_runtime(
    selected_vendor: &str,
    selected_model: &str,
    vendor_base_url: &str,
    vendor_api_key: &str,
    vendor_api_format: Option<&str>,
) -> Result<Arc<LlmProviderRuntime>, String> {
    let provider_type = llm_provider_type_for_vendor(selected_vendor, vendor_api_format);
    let config = claw_core::config::LlmProviderConfig {
        name: format!("vendor-{}", selected_vendor.trim().to_ascii_lowercase()),
        provider_type,
        base_url: vendor_base_url.trim().to_string(),
        api_key: vendor_api_key.trim().to_string(),
        model: selected_model.trim().to_string(),
        context_window_tokens: None,
        priority: 1,
        timeout_seconds: 20,
        max_concurrency: 1,
        params: claw_core::config::LlmProviderParams::default(),
    };
    let client = crate::providers::build_llm_http_client(config.timeout_seconds)
        .map_err(|err| format!("build llm test client failed: {err}"))?;
    Ok(Arc::new(LlmProviderRuntime {
        config,
        pricing: None,
        client,
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    }))
}

fn llm_runtime_differs(
    runtime_vendor: &str,
    runtime_model: &str,
    runtime_provider_type: &str,
    runtime_base_url: &str,
    runtime_api_key: &str,
    selected_vendor: &str,
    selected_model: &str,
    saved_provider_type: &str,
    saved_base_url: &str,
    saved_api_key: &str,
) -> bool {
    runtime_vendor.trim() != selected_vendor.trim()
        || runtime_model.trim() != selected_model.trim()
        || (llm_vendor_supports_api_format(selected_vendor)
            && runtime_provider_type.trim() != saved_provider_type.trim())
        || runtime_base_url.trim() != saved_base_url.trim()
        || runtime_api_key.trim() != saved_api_key.trim()
}

fn llm_restart_required(
    state: &AppState,
    parsed: &toml::Value,
    selected_vendor: &str,
    selected_model: &str,
) -> bool {
    let Some(provider) = state.core.llm_providers.first() else {
        return true;
    };
    let runtime_vendor = provider
        .config
        .name
        .strip_prefix("vendor-")
        .unwrap_or(provider.config.name.as_str());
    let (saved_base_url, saved_api_key, saved_provider_type) =
        saved_llm_vendor_runtime_fields(parsed, selected_vendor.trim());
    llm_runtime_differs(
        runtime_vendor,
        &provider.config.model,
        &provider.config.provider_type,
        &provider.config.base_url,
        &provider.config.api_key,
        selected_vendor,
        selected_model,
        &saved_provider_type,
        &saved_base_url,
        &saved_api_key,
    )
}

fn skills_restart_required(runtime_visible: &[String], effective_visible: &[String]) -> bool {
    let mut runtime_sorted = runtime_visible.to_vec();
    runtime_sorted.sort_unstable();
    let mut effective_sorted = effective_visible.to_vec();
    effective_sorted.sort_unstable();
    runtime_sorted != effective_sorted
}

fn collect_skills_baseline(value: &toml::Value, state: &AppState) -> Vec<String> {
    value
        .get("skills")
        .and_then(|v| v.get("skills_list"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| state.resolve_canonical_skill_name(s))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn collect_skill_switches(value: &toml::Value, state: &AppState) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    let Some(tbl) = value
        .get("skills")
        .and_then(|v| v.get("skill_switches"))
        .and_then(|v| v.as_table())
    else {
        return out;
    };
    for (k, v) in tbl {
        let canonical = state.resolve_canonical_skill_name(k);
        if hide_skill_in_ui(state, &canonical) {
            continue;
        }
        if let Some(b) = v.as_bool() {
            out.insert(canonical, b);
        }
    }
    out
}

fn registry_tool_capability_names(state: &AppState) -> Vec<String> {
    let mut out = state
        .get_skills_registry()
        .as_ref()
        .map(|registry| {
            registry
                .all_names()
                .into_iter()
                .filter(|name| {
                    !hide_skill_in_ui(state, name)
                        && registry
                            .planner_kind(name)
                            .map(|kind| kind == PlannerCapabilityKind::Tool)
                            .unwrap_or(false)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.sort_unstable();
    out
}

fn compute_effective_enabled(
    baseline: &[String],
    switches: &BTreeMap<String, bool>,
    state: &AppState,
) -> Vec<String> {
    let mut set: BTreeMap<String, bool> = BTreeMap::new();
    for skill in baseline {
        set.insert(state.resolve_canonical_skill_name(skill), true);
    }
    if let Some(registry) = state.get_skills_registry() {
        for skill in registry.enabled_names() {
            set.insert(state.resolve_canonical_skill_name(&skill), true);
        }
    }
    for (k, v) in switches {
        if *v {
            set.insert(state.resolve_canonical_skill_name(k), true);
        } else {
            set.remove(&state.resolve_canonical_skill_name(k));
        }
    }
    set.into_keys().collect()
}

fn render_switches_inline_table(switches: &BTreeMap<String, bool>) -> String {
    if switches.is_empty() {
        return "skill_switches = {}".to_string();
    }
    let pairs = switches
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("skill_switches = {{ {pairs} }}")
}

fn upsert_skill_switches_line(raw: &str, rendered_line: &str) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let mut in_skills = false;
    let mut inserted_or_replaced = false;
    let mut skills_section_seen = false;
    let mut insert_index_in_skills: Option<usize> = None;
    let mut skills_section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == "[skills]" {
            in_skills = true;
            skills_section_seen = true;
            insert_index_in_skills = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != "[skills]" {
            if in_skills {
                skills_section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_skills && trimmed.starts_with("skill_switches") && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
        if in_skills && insert_index_in_skills.is_none() && !trimmed.is_empty() {
            insert_index_in_skills = Some(idx);
        }
        if in_skills && trimmed.starts_with("skills_list") && insert_index_in_skills.is_none() {
            insert_index_in_skills = Some(idx);
        }
    }

    if !inserted_or_replaced && skills_section_seen {
        let idx = insert_index_in_skills
            .or(skills_section_end)
            .unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn render_toml_string_array(key: &str, values: &[String]) -> Vec<String> {
    if values.is_empty() {
        return vec![format!("{key} = []")];
    }
    let mut lines = vec![format!("{key} = [")];
    for value in values {
        lines.push(format!("    {},", toml_string_literal(value)));
    }
    lines.push("]".to_string());
    lines
}

fn array_block_end(lines: &[String], start: usize) -> usize {
    let mut depth: isize = 0;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        depth += line.matches('[').count() as isize;
        depth -= line.matches(']').count() as isize;
        if depth <= 0 && line.contains(']') {
            return idx;
        }
    }
    start
}

fn ensure_string_array_contains_in_section(
    raw: &str,
    section_name: &str,
    key: &str,
    current_values: &[String],
    required_value: &str,
) -> String {
    let required = required_value.trim();
    if required.is_empty() || current_values.iter().any(|value| value == required) {
        return raw.to_string();
    }

    let mut values = current_values.to_vec();
    values.push(required.to_string());
    let rendered = render_toml_string_array(key, &values);
    let section_header = format!("[{section_name}]");
    let mut lines: Vec<String> = raw.lines().map(|line| line.to_string()).collect();
    let Some(section_start) = lines.iter().position(|line| line.trim() == section_header) else {
        return raw.to_string();
    };
    let section_end = lines
        .iter()
        .enumerate()
        .skip(section_start + 1)
        .find(|(_, line)| {
            let trimmed = line.trim();
            trimmed.starts_with('[') && trimmed.ends_with(']')
        })
        .map(|(idx, _)| idx)
        .unwrap_or(lines.len());
    let target_start = (section_start + 1..section_end).find(|idx| {
        let trimmed = lines[*idx].trim_start();
        if trimmed.starts_with('#') {
            return false;
        }
        trimmed
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    });

    if let Some(start) = target_start {
        let end = array_block_end(&lines, start).min(section_end.saturating_sub(1));
        lines.splice(start..=end, rendered);
    } else {
        let mut insert_at = section_end;
        while insert_at > section_start + 1 && lines[insert_at - 1].trim().is_empty() {
            insert_at -= 1;
        }
        lines.splice(insert_at..insert_at, rendered);
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

async fn get_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, v)) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let baseline = collect_skills_baseline(&parsed, &state);
    let switches = collect_skill_switches(&parsed, &state);
    let mut baseline_visible = baseline
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    baseline_visible.sort_unstable();
    let mut runtime_visible = state
        .get_skills_list()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    runtime_visible.sort_unstable();
    let managed = {
        let mut set: BTreeMap<String, bool> = BTreeMap::new();
        for s in &baseline_visible {
            set.insert(s.clone(), true);
        }
        for s in switches.keys() {
            set.insert(s.clone(), true);
        }
        for s in runtime_visible.iter() {
            set.insert(s.clone(), true);
        }
        set.into_keys().collect::<Vec<_>>()
    };
    let mut effective = compute_effective_enabled(&baseline, &switches, &state);
    effective.retain(|s| !hide_skill_in_ui(&state, s));
    let restart_required = skills_restart_required(&runtime_visible, &effective);
    let base_skill_names: Vec<String> = claw_core::config::base_skill_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let core_skill_names: Vec<String> = claw_core::config::core_skills_always_enabled()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .map(|s| s.to_string())
        .collect();
    let tool_skill_names = registry_tool_capability_names(&state);
    let locked_skill_names = {
        let mut set = BTreeSet::new();
        for name in &core_skill_names {
            set.insert(name.clone());
        }
        for name in &tool_skill_names {
            set.insert(name.clone());
        }
        set.into_iter().collect::<Vec<_>>()
    };
    let external_skill_names = state
        .get_skills_registry()
        .as_ref()
        .map(|registry| {
            registry
                .all_names()
                .into_iter()
                .filter(|name| {
                    !hide_skill_in_ui(&state, name)
                        && registry
                            .get(name)
                            .map(|entry| entry.kind == SkillKind::External)
                            .unwrap_or(false)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let skill_items = managed
        .iter()
        .map(|name| build_skill_list_item(&state, name))
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skills_list": baseline_visible,
                "skill_switches": switches,
                "managed_skills": managed,
                "base_skill_names": base_skill_names,
                "core_skill_names": core_skill_names,
                "tool_skill_names": tool_skill_names,
                "locked_skill_names": locked_skill_names,
                "external_skill_names": external_skill_names,
                "skill_items": skill_items,
                "effective_enabled_skills_preview": effective,
                "runtime_enabled_skills": runtime_visible,
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn get_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, v)) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let llm = parsed.get("llm").and_then(|v| v.as_table());
    let selected_vendor = llm
        .and_then(|tbl| tbl.get("selected_vendor"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let selected_model = llm
        .and_then(|tbl| tbl.get("selected_model"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let vendors = collect_llm_vendor_info(&parsed);
    let restart_required = llm_restart_required(&state, &parsed, &selected_vendor, &selected_model);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "selected_vendor": selected_vendor,
                "selected_model": selected_model,
                "vendors": vendors,
                "runtime": current_runtime_llm_info(&state),
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn update_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateLlmConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let selected_vendor = req.selected_vendor.trim().to_ascii_lowercase();
    let selected_model = req.selected_model.trim().to_string();
    if selected_vendor.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_vendor is required".to_string()),
            }),
        );
    }
    if selected_model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_model is required".to_string()),
            }),
        );
    }

    let (raw, parsed) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let vendors = collect_llm_vendor_info(&parsed);
    let Some(vendor_info) = vendors.iter().find(|item| {
        item.get("name")
            .and_then(|v| v.as_str())
            .map(|name| name.eq_ignore_ascii_case(&selected_vendor))
            .unwrap_or(false)
    }) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unsupported vendor: {selected_vendor}")),
            }),
        );
    };

    let allowed_models = vendor_info
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let vendor_base_url = req.vendor_base_url.as_deref().map(str::trim).unwrap_or("");
    if vendor_base_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("vendor_base_url is required".to_string()),
            }),
        );
    }

    let updated_vendor = upsert_string_key_in_section(
        &raw,
        "llm",
        "selected_vendor",
        &format!("selected_vendor = {:?}", selected_vendor),
    );
    let updated_raw = upsert_string_key_in_section(
        &updated_vendor,
        "llm",
        "selected_model",
        &format!("selected_model = {:?}", selected_model),
    );
    let updated_vendor_base_url = upsert_string_key_in_section(
        &updated_raw,
        &format!("llm.{selected_vendor}"),
        "base_url",
        &format!("base_url = {:?}", vendor_base_url),
    );
    let updated_vendor_model = upsert_string_key_in_section(
        &updated_vendor_base_url,
        &format!("llm.{selected_vendor}"),
        "model",
        &format!("model = {:?}", selected_model),
    );
    let updated_vendor_models = ensure_string_array_contains_in_section(
        &updated_vendor_model,
        &format!("llm.{selected_vendor}"),
        "models",
        &allowed_models,
        &selected_model,
    );
    let vendor_api_key = req.vendor_api_key.as_deref().map(str::trim).unwrap_or("");
    let updated_api_key = upsert_string_key_in_section(
        &updated_vendor_models,
        &format!("llm.{selected_vendor}"),
        "api_key",
        &format!("api_key = {:?}", vendor_api_key),
    );
    let final_updated = if llm_vendor_supports_api_format(&selected_vendor) {
        let vendor_api_format = normalize_llm_api_format(req.vendor_api_format.as_deref());
        upsert_string_key_in_section(
            &updated_api_key,
            &format!("llm.{selected_vendor}"),
            "api_format",
            &format!("api_format = {:?}", vendor_api_format),
        )
    } else {
        updated_api_key
    };
    if let Err(err) = write_runtime_config_file(&state, &final_updated) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write llm config failed: {err}")),
            }),
        );
    }
    let updated_parsed = toml::from_str::<toml::Value>(&final_updated).unwrap_or(parsed);
    let restart_required =
        llm_restart_required(&state, &updated_parsed, &selected_vendor, &selected_model);

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "selected_vendor": selected_vendor,
                "selected_model": selected_model,
                "runtime": current_runtime_llm_info(&state),
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn test_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateLlmConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let selected_vendor = req.selected_vendor.trim().to_ascii_lowercase();
    let selected_model = req.selected_model.trim().to_string();
    if selected_vendor.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_vendor is required".to_string()),
            }),
        );
    }
    if selected_model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_model is required".to_string()),
            }),
        );
    }

    let parsed = match read_skill_config_file(&state) {
        Ok((_, parsed)) => parsed,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let vendors = collect_llm_vendor_info(&parsed);
    let Some(_vendor_info) = vendors.iter().find(|item| {
        item.get("name")
            .and_then(|v| v.as_str())
            .map(|name| name.eq_ignore_ascii_case(&selected_vendor))
            .unwrap_or(false)
    }) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unsupported vendor: {selected_vendor}")),
            }),
        );
    };

    let vendor_base_url = req.vendor_base_url.as_deref().map(str::trim).unwrap_or("");
    if vendor_base_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("vendor_base_url is required".to_string()),
            }),
        );
    }
    let vendor_api_key = req.vendor_api_key.as_deref().map(str::trim).unwrap_or("");
    let provider = match build_llm_test_runtime(
        &selected_vendor,
        &selected_model,
        vendor_base_url,
        vendor_api_key,
        req.vendor_api_format.as_deref(),
    ) {
        Ok(provider) => provider,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err),
                }),
            );
        }
    };

    match crate::call_provider_with_retry(provider.clone(), LLM_CONNECTIVITY_TEST_PROMPT).await {
        Ok(resp) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "success": true,
                    "vendor": selected_vendor,
                    "model": selected_model,
                    "provider_type": provider.config.provider_type,
                    "message_key": "clawd.msg.provider_connection_test_ok",
                    "message_args": {
                        "provider_name": provider.config.name,
                    },
                    "response_text": resp.text,
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("llm connectivity test failed: {err}")),
            }),
        ),
    }
}

async fn update_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateSkillsConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let (raw, parsed) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let baseline = collect_skills_baseline(&parsed, &state);
    let core_skills = claw_core::config::core_skills_always_enabled();
    let tool_skill_names = registry_tool_capability_names(&state)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut switches = BTreeMap::new();
    for (k, v) in req.skill_switches {
        let skill = state.resolve_canonical_skill_name(k.trim());
        if skill.is_empty() || hide_skill_in_ui(&state, &skill) {
            continue;
        }
        let is_core = core_skills.iter().any(|s| *s == skill);
        let is_tool = tool_skill_names.contains(&skill);
        switches.insert(skill, if is_core || is_tool { true } else { v });
    }
    let rendered = render_switches_inline_table(&switches);
    let updated = upsert_skill_switches_line(&raw, &rendered);
    if let Err(err) = write_runtime_config_file(&state, &updated) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills config failed: {err}")),
            }),
        );
    }
    let effective = compute_effective_enabled(&baseline, &switches, &state);
    let mut effective_visible = effective
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    effective_visible.sort_unstable();
    let mut runtime_visible = state
        .get_skills_list()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    runtime_visible.sort_unstable();
    let restart_required = skills_restart_required(&runtime_visible, &effective_visible);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skill_switches": switches,
                "effective_enabled_skills_preview": effective,
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn uninstall_external_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UninstallExternalSkillRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let skill_name = state.resolve_canonical_skill_name(req.skill_name.trim());
    if skill_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skill_name is required".to_string()),
            }),
        );
    }

    let Some(registry) = state.get_skills_registry() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skills registry is not available".to_string()),
            }),
        );
    };
    let Some(entry) = registry.get(&skill_name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unknown skill: {skill_name}")),
            }),
        );
    };
    if entry.kind != SkillKind::External {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only imported external skills can be uninstalled here".to_string()),
            }),
        );
    }

    let registry_raw = match read_skills_registry_file(&state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {err}")),
                }),
            );
        }
    };
    let (updated_registry, removed_from_registry) =
        remove_skill_registry_block(&registry_raw, &skill_name);
    if !removed_from_registry {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("skill registry block not found for {skill_name}")),
            }),
        );
    }
    if let Err(err) = write_skills_registry_file(&state, &updated_registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills registry failed: {err}")),
            }),
        );
    }

    let mut removed_bundle = false;
    if let Some(bundle_rel) = entry.external_bundle_dir.as_deref() {
        let bundle_path = if Path::new(bundle_rel).is_absolute() {
            PathBuf::from(bundle_rel)
        } else {
            state.skill_rt.workspace_root.join(bundle_rel)
        };
        let allowed_root = state.skill_rt.workspace_root.join("third_party");
        if bundle_path.starts_with(&allowed_root) && bundle_path.exists() {
            match std::fs::remove_dir_all(&bundle_path) {
                Ok(_) => removed_bundle = true,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("remove imported bundle failed: {err}")),
                        }),
                    );
                }
            }
        }
    }

    let mut removed_prompt = false;
    let registry_prompt_rel_path = entry.prompt_file.trim();
    if !registry_prompt_rel_path.is_empty() {
        let prompt_body_path = if let Some(prompt_body_rel) =
            prompt_layers::canonical_skill_prompt_body_rel_path(registry_prompt_rel_path)
        {
            state.skill_rt.workspace_root.join(prompt_body_rel)
        } else if Path::new(registry_prompt_rel_path).is_absolute() {
            PathBuf::from(registry_prompt_rel_path)
        } else {
            state.skill_rt.workspace_root.join(registry_prompt_rel_path)
        };
        match remove_managed_prompt_file(&prompt_body_path) {
            Ok(value) => removed_prompt = value,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("remove prompt file failed: {err}")),
                    }),
                );
            }
        }
    }

    let (runtime_raw, _) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let updated_runtime = remove_runtime_skill_switch(&runtime_raw, &state, &skill_name);
    if let Err(err) = write_runtime_config_file(&state, &updated_runtime) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills config failed: {err}")),
            }),
        );
    }

    let reload = match reload_skill_views(&state) {
        Ok(result) => result,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("reload skill views failed: {err}")),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": skill_name,
                "removed_bundle": removed_bundle,
                "removed_prompt": removed_prompt,
                "reload": reload,
            })),
            error: None,
        }),
    )
}
