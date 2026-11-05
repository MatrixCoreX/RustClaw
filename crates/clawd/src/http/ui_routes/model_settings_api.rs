#[derive(Clone, Copy)]
enum ModelSettingsOperation {
    ReadModels,
    SaveModels,
    ReadKeys,
    SaveKeys,
}

impl ModelSettingsOperation {
    fn writes(self) -> bool {
        matches!(self, Self::SaveModels | Self::SaveKeys)
    }
    fn failure_code(self) -> &'static str {
        match self {
            Self::ReadModels => "model_config_read_failed",
            Self::SaveModels => "model_config_write_failed",
            Self::ReadKeys => "provider_keys_read_failed",
            Self::SaveKeys => "provider_keys_write_failed",
        }
    }
}

fn authorize_model_settings<T>(
    state: &AppState,
    headers: &HeaderMap,
    operation: ModelSettingsOperation,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    let result = if operation.writes() {
        require_ui_admin(state, headers)
    } else {
        require_ui_identity(state, headers)
    };
    result.map(|_| ()).map_err(|(status, Json(response))| {
        (
            status,
            Json(ApiResponse {
                ok: response.ok,
                data: None,
                error: response.error,
            }),
        )
    })
}

async fn model_settings_request<T, F>(
    state: AppState,
    headers: HeaderMap,
    operation: ModelSettingsOperation,
    execute: F,
) -> (StatusCode, Json<ApiResponse<T>>)
where
    T: Serialize + Send + 'static,
    F: FnOnce(&AppState) -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(move || {
        if let Err(response) = authorize_model_settings(&state, &headers, operation) {
            return response;
        }
        crate::configuration_files::with_locked_files(&model_settings_lock_paths(&state), || {
            // Recheck after waiting for another request; never reuse an old role or
            // enabled-key result as authorization for the actual filesystem work.
            if let Err(response) = authorize_model_settings(&state, &headers, operation) {
                return response;
            }
            match execute(&state) {
                Ok(data) => (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(data),
                        error: None,
                    }),
                ),
                Err(error) => {
                    let failure = error.downcast_ref::<ModelSettingsFailure>();
                    let code = failure
                        .map(|failure| failure.error_code)
                        .unwrap_or(operation.failure_code());
                    tracing::warn!(
                        error_code = code,
                        config_file = failure.map(|failure| failure.file),
                        "model_settings_operation_failed"
                    );
                    crate::api_err(
                        failure
                            .map(|value| value.status)
                            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        code,
                    )
                }
            }
        })
        .unwrap_or_else(|_| {
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "model_settings_lock_unavailable",
            )
        })
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                panicked = error.is_panic(),
                cancelled = error.is_cancelled(),
                "model_settings_operation_join_failed"
            );
            crate::api_err(StatusCode::INTERNAL_SERVER_ERROR, operation.failure_code())
        }
    }
}

async fn get_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    model_settings_request(
        state,
        headers,
        ModelSettingsOperation::ReadModels,
        read_model_config,
    )
    .await
}

async fn get_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    model_settings_request(
        state,
        headers,
        ModelSettingsOperation::ReadKeys,
        read_provider_keys,
    )
    .await
}

async fn update_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ModelConfigUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    model_settings_request(
        state,
        headers,
        ModelSettingsOperation::SaveModels,
        move |state| {
            write_model_config(state, &req)?;
            read_model_config(state)
        },
    )
    .await
}

async fn update_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProviderKeysResponse>,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    model_settings_request(
        state,
        headers,
        ModelSettingsOperation::SaveKeys,
        move |state| {
            write_provider_keys(state, &req)?;
            read_provider_keys(state)
        },
    )
    .await
}
