async fn configuration_read_request(
    state: AppState,
    headers: HeaderMap,
    read: fn(State<AppState>, HeaderMap) -> (StatusCode, Json<ApiResponse<Value>>),
    error_code: &'static str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    // Authorization, active-path resolution, filesystem and admission reads
    // all belong to the same operation. Never cache its authorization result.
    match tokio::task::spawn_blocking(move || read(State(state), headers)).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                error_code,
                worker_panicked = error.is_panic(),
                worker_cancelled = error.is_cancelled(),
                "configuration_read_worker_failed"
            );
            crate::api_err(StatusCode::INTERNAL_SERVER_ERROR, error_code)
        }
    }
}

async fn get_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    configuration_read_request(
        state,
        headers,
        read_skills_config,
        "skills_config_read_failed",
    )
    .await
}

async fn get_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    configuration_read_request(state, headers, read_llm_config, "llm_config_read_failed").await
}
