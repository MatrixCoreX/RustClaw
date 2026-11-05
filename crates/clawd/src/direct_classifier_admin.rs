fn classifier_source_allowed(source: &str) -> bool {
    let normalized = source.trim().to_ascii_lowercase();
    !normalized.is_empty()
}

fn channel_kind_label(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Telegram => "telegram",
        ChannelKind::Whatsapp => "whatsapp",
        ChannelKind::Ui => "ui",
        ChannelKind::Wechat => "wechat",
        ChannelKind::Feishu => "feishu",
        ChannelKind::Lark => "lark",
    }
}

fn require_auth_identity_for_api<T: Serialize>(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<T>>)> {
    let Some(raw_key) = auth_key_from_headers(headers)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(api_err::<T>(StatusCode::UNAUTHORIZED, "auth_key_required"));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(api_err::<T>(StatusCode::UNAUTHORIZED, "auth_key_invalid")),
        Err(err) => {
            error!("resolve auth identity failed: {}", err);
            Err(api_err::<T>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Auth lookup failed",
            ))
        }
    }
}

async fn classify_direct(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DirectClassifyRequest>,
) -> (StatusCode, Json<ApiResponse<DirectClassifyResponse>>) {
    let identity = match require_auth_identity_for_api(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let source = req.source.trim().to_ascii_lowercase();
    if !classifier_source_allowed(&source) {
        return api_err::<DirectClassifyResponse>(
            StatusCode::BAD_REQUEST,
            "source is required for direct classifier",
        );
    }
    let text = req.text.trim();
    if text.is_empty() {
        return api_err::<DirectClassifyResponse>(StatusCode::BAD_REQUEST, "text is required");
    }
    let channel_kind = req.channel.unwrap_or(ChannelKind::Ui);
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: format!("direct-classify-{}", Uuid::new_v4()),
        user_id: identity.user_id,
        chat_id: req.chat_id.unwrap_or(identity.chat_id),
        user_key: Some(identity.user_key.clone()),
        channel: channel_kind_label(channel_kind).to_string(),
        external_user_id: normalize_external_id_opt(req.external_user_id.as_deref()),
        external_chat_id: normalize_external_id_opt(req.external_chat_id.as_deref()),
        kind: "ask".to_string(),
        payload_json: json!({
            "text": text,
            "source": source
        })
        .to_string(),
    };
    info!(
        "direct_classifier_request task_id={} source={} user_id={} chat_id={}",
        task.task_id, source, task.user_id, task.chat_id
    );
    let result = finalize::run_direct_classifier_reply(&state, &task, text).await;
    state.clear_task_llm_call_count(&task.task_id);
    match result {
        Ok(reply) => api_ok(DirectClassifyResponse {
            text: reply.text.trim().to_string(),
        }),
        Err(err) => {
            warn!(
                "direct classifier failed: task_id={} source={} err={}",
                task.task_id, source, err
            );
            api_err::<DirectClassifyResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Direct classifier failed",
            )
        }
    }
}

#[derive(Debug, Serialize)]
struct ActiveTaskItem {
    index: usize,
    task_id: String,
    kind: String,
    status: String,
    execution_state: String,
    channel: String,
    source_user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    external_user_id: Option<String>,
    summary: String,
    age_seconds: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct TaskHistoryItem {
    task_id: String,
    kind: String,
    status: String,
    channel: String,
    source_user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    external_user_id: Option<String>,
    summary: String,
    created_at_ts: i64,
    updated_at_ts: i64,
    duration_seconds: i64,
}

async fn reload_skills_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    skill_reload_request(state, headers, |state| {
        reload_skill_views(state).map(|result| serde_json::to_value(result).unwrap_or_default())
    })
    .await
}

async fn skill_reload_request<F>(
    state: AppState,
    headers: HeaderMap,
    reload: F,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>)
where
    F: FnOnce(&AppState) -> Result<serde_json::Value, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(move || {
        if let Err(response) = http::ui_routes::require_ui_admin(&state, &headers) {
            return response;
        }
        match reload(&state) {
            Ok(result) => api_ok(result),
            Err(error) => {
                warn!(%error, "skill_runtime_reload_failed");
                api_err(StatusCode::INTERNAL_SERVER_ERROR, "skill_runtime_reload_failed")
            }
        }
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            warn!(%error, "skill_runtime_reload_worker_failed");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "skill_runtime_reload_worker_failed")
        }
    }
}

#[cfg(test)]
#[path = "runtime/admin_skill_reload_tests.rs"]
mod admin_skill_reload_tests;
