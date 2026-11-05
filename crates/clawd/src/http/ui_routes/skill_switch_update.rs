async fn update_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateSkillsConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    // Keep fresh authorization and the whole write/reload operation together
    // off the asynchronous executor, including admission lock contention.
    match tokio::task::spawn_blocking(move || write_skills_config(State(state), headers, Json(req)))
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                worker_panicked = error.is_panic(),
                worker_cancelled = error.is_cancelled(),
                "skill_switch_worker_failed"
            );
            crate::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "skills_config_write_failed",
            )
        }
    }
}
