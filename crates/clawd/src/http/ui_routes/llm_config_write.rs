async fn update_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateLlmConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    update_llm_config_with_writer(state, headers, req, write_runtime_config_file)
}

// Private publication seam for isolated transaction-failure controls.
fn update_llm_config_with_writer(
    state: AppState,
    headers: HeaderMap,
    req: UpdateLlmConfigRequest,
    publish: impl FnOnce(&AppState, &str) -> std::io::Result<()>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_admin(&state, &headers) {
        return resp;
    }
    let inline_api_key = req.vendor_api_key.as_deref().map(str::trim).unwrap_or("");
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
    let vendors = collect_llm_vendor_info_for_state(&parsed, &state);
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
    let device_key_enrollment =
        matches_hosted_relay_preset(&parsed, &selected_vendor, &selected_model, vendor_base_url);
    if device_key_enrollment && !inline_api_key.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("hosted_relay_manual_key_not_allowed".to_string()),
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
    let updated_api_key = upsert_string_key_in_section(
        &updated_vendor_models,
        &format!("llm.{selected_vendor}"),
        "api_key",
        "api_key = \"\"",
    );
    let mut final_updated = if llm_vendor_supports_api_format(&selected_vendor) {
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
    if device_key_enrollment {
        for class_name in ["default", "fast", "reasoning"] {
            final_updated = upsert_string_key_in_section(
                &final_updated,
                &format!("llm.model_classes.{class_name}"),
                "provider",
                &format!("provider = {:?}", selected_vendor),
            );
            final_updated = upsert_string_key_in_section(
                &final_updated,
                &format!("llm.model_classes.{class_name}"),
                "model",
                &format!("model = {:?}", selected_model),
            );
        }
    }
    if !inline_api_key.is_empty() {
        if claw_core::secrets::model_environment::save(
            &state.skill_rt.workspace_root,
            &selected_vendor,
            inline_api_key,
        )
        .is_err()
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("llm_api_key_environment_write_failed".to_string()),
                }),
            );
        }
    }
    if let Err(err) = publish(&state, &final_updated) {
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
