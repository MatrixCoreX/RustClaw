async fn get_model_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    // Authentication also uses the synchronous DB pool. Keep the whole read on
    // one blocking worker, with fresh authorization and no cross-request cache.
    match tokio::task::spawn_blocking(move || read_model_catalog(&state, &headers)).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                panicked = error.is_panic(),
                cancelled = error.is_cancelled(),
                "model_catalog_read_join_failed"
            );
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "model_catalog_unavailable",
            )
        }
    }
}

fn read_model_catalog(
    state: &AppState,
    headers: &HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(state, headers) {
        return resp;
    }
    match crate::configuration_files::read_model_catalog(state) {
        Ok(catalog) => {
            let mut data = serde_json::to_value(catalog).unwrap_or_else(|_| json!({}));
            if let Some(object) = data.as_object_mut() {
                object.insert(
                    "last_guard_status".to_string(),
                    read_model_catalog_guard_status(&state.skill_rt.workspace_root),
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
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: Some(model_catalog_failure_data(&error)),
                error: Some("model_catalog_unavailable".to_string()),
            }),
        ),
    }
}

fn model_catalog_failure_data(error: &claw_core::model_catalog::ModelCatalogError) -> Value {
    // TOML Display includes the offending source line. Never format that error
    // (or its path) into either the public catalog or teaching trace.
    use claw_core::model_catalog::ModelCatalogError;
    let failure_code = match error {
        ModelCatalogError::Read { .. } => "model_catalog_read_failed",
        ModelCatalogError::Parse { .. } => "model_catalog_parse_failed",
    };
    json!({
        "error_code": "model_catalog_unavailable",
        "failure_code": failure_code,
    })
}

fn read_model_catalog_guard_status(root: &Path) -> Value {
    let gate_root = root.join("logs/agent_parity_gate");
    let Ok(entries) = std::fs::read_dir(&gate_root) else {
        return json!({
            "available": false,
            "status": "missing",
        });
    };

    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let candidate = entry.path().join("chinese_model_catalog.json");
        if !candidate.is_file() {
            continue;
        }
        let modified = candidate
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        if latest
            .as_ref()
            .is_none_or(|(previous, _)| modified > *previous)
        {
            latest = Some((modified, candidate));
        }
    }

    let Some((modified, path)) = latest else {
        return json!({
            "available": false,
            "status": "missing",
        });
    };
    let modified_ts = modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok());
    let path_token = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();

    let Ok(raw) = std::fs::read_to_string(&path) else {
        return json!({
            "available": false,
            "status": "read_failed",
            "path": path_token,
            "modified_ts": modified_ts,
        });
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return json!({
            "available": false,
            "status": "parse_failed",
            "path": path_token,
            "modified_ts": modified_ts,
        });
    };
    json!({
        "available": true,
        "status": parsed.get("status").and_then(Value::as_str).unwrap_or("unknown"),
        "finding_count": parsed.get("finding_count").and_then(Value::as_u64).unwrap_or(0),
        "path": path_token,
        "modified_ts": modified_ts,
    })
}
