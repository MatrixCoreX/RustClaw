async fn get_auth_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    match list_auth_keys(&state) {
        Ok(rows) => {
            let list: Vec<Value> = rows
                .into_iter()
                .filter(|row| {
                    identity.role.eq_ignore_ascii_case("admin") || row.user_key == identity.user_key
                })
                .map(|row| {
                    json!({
                        "key_id": row.key_id,
                        "user_key": row.user_key,
                        "user_key_masked": row.user_key_masked,
                        "role": row.role,
                        "enabled": row.enabled != 0,
                        "created_at": row.created_at,
                        "last_used_at": row.last_used_at,
                        "webd_username": row.webd_username,
                        "current_key": row.user_key == identity.user_key,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({ "keys": list })),
                    error: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("list auth keys failed: {err}")),
            }),
        ),
    }
}

async fn get_auth_key_full_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can reveal auth keys".to_string()),
            }),
        );
    }

    match get_auth_key_value_by_id(&state, key_id) {
        Ok(Some(user_key)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("get auth key failed: {err}")),
            }),
        ),
    }
}

fn clamp_feishu_bind_ttl_seconds(raw: Option<u64>) -> u64 {
    raw.unwrap_or(FEISHU_BIND_SESSION_DEFAULT_TTL_SECONDS)
        .clamp(
            FEISHU_BIND_SESSION_MIN_TTL_SECONDS,
            FEISHU_BIND_SESSION_MAX_TTL_SECONDS,
        )
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationInitResponse {
    #[serde(default)]
    supported_auth_methods: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationBeginResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expire_in: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationUserInfo {
    #[serde(default)]
    open_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuOfficialRegistrationPollResponse {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    user_info: Option<FeishuOfficialRegistrationUserInfo>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

fn feishu_accounts_base_url() -> String {
    std::env::var("RUSTCLAW_FEISHU_ACCOUNTS_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| FEISHU_OFFICIAL_ACCOUNTS_BASE_URL.to_string())
}

async fn call_feishu_official_registration<T: DeserializeOwned>(
    state: &AppState,
    params: &[(&str, &str)],
) -> anyhow::Result<T> {
    let url = format!("{}/oauth/v1/app/registration", feishu_accounts_base_url());
    let resp = state.core.http_client.post(url).form(params).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    serde_json::from_str::<T>(&body).map_err(|err| {
        anyhow::anyhow!(
            "decode feishu registration response failed: status={} body={} err={}",
            status,
            body,
            err
        )
    })
}

async fn begin_feishu_official_registration(
    state: &AppState,
) -> anyhow::Result<FeishuOfficialRegistrationBeginResponse> {
    let init = call_feishu_official_registration::<FeishuOfficialRegistrationInitResponse>(
        state,
        &[("action", "init")],
    )
    .await?;
    if !init
        .supported_auth_methods
        .iter()
        .any(|method| method == "client_secret")
    {
        anyhow::bail!("feishu registration does not support client_secret auth");
    }
    let begin = call_feishu_official_registration::<FeishuOfficialRegistrationBeginResponse>(
        state,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;
    if begin.device_code.trim().is_empty() || begin.verification_uri_complete.trim().is_empty() {
        anyhow::bail!("feishu registration did not return a device_code or verification url");
    }
    Ok(begin)
}

async fn poll_feishu_official_registration(
    state: &AppState,
    device_code: &str,
) -> anyhow::Result<FeishuOfficialRegistrationPollResponse> {
    call_feishu_official_registration::<FeishuOfficialRegistrationPollResponse>(
        state,
        &[("action", "poll"), ("device_code", device_code)],
    )
    .await
}

fn feishu_entry_url_for_app_id(app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "https://applink.feishu.cn/client/bot/open?appId={trimmed}"
    ))
}

fn feishu_bind_entry_url(
    state: &AppState,
    session: Option<&PendingChannelBindSession>,
) -> Option<String> {
    let config = load_feishu_config_response(state, None).ok()?;
    if config.bind_ready {
        if let Some(entry_url) = feishu_entry_url_for_app_id(&config.app_id) {
            return Some(entry_url);
        }
    }
    session
        .and_then(|session| session.install_verification_url.clone())
        .filter(|url| !url.trim().is_empty())
}

fn feishu_bind_session_response(
    state: &AppState,
    session: PendingChannelBindSession,
) -> FeishuBindSessionStatusResponse {
    let entry_url = feishu_bind_entry_url(state, Some(&session));
    FeishuBindSessionStatusResponse {
        session_id: session.id,
        channel: session.channel,
        bind_token: session.bind_token,
        status: session.status,
        external_user_id: session.external_user_id,
        external_chat_id: session.external_chat_id,
        error_text: session.error_text,
        created_at: session.created_at,
        updated_at: session.updated_at,
        expires_at: session.expires_at,
        entry_url,
    }
}

fn maybe_expire_feishu_bind_session(
    db: &mut rusqlite::Connection,
    session: PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    if matches!(session.status.as_str(), "pending" | "detected") {
        let expires_at = session.expires_at.parse::<i64>().unwrap_or_default();
        if expires_at > 0 && expires_at <= current_unix_ts() {
            return mark_pending_channel_bind_session_expired(db, session.id);
        }
    }
    Ok(session)
}

fn write_feishu_generated_credentials(
    state: &AppState,
    app_id: &str,
    app_secret: &str,
) -> anyhow::Result<()> {
    let raw = read_feishu_config_raw(state)?;
    let output = update_feishu_config_raw_preserving_format(&raw, app_id, app_secret);
    write_workspace_and_mounted_file(
        &state.skill_rt.workspace_root,
        "configs/channels/feishu.toml",
        &output,
    )?;
    Ok(())
}

async fn start_service_if_needed(state: &AppState, service: &str) -> anyhow::Result<()> {
    if service_is_running(service) {
        return Ok(());
    }
    let profile = std::env::var("RUSTCLAW_START_PROFILE")
        .ok()
        .filter(|v| matches!(v.as_str(), "debug" | "release"))
        .unwrap_or_else(|| runtime_profile_default().to_string());
    let script_name =
        service_start_script(service).ok_or_else(|| anyhow::anyhow!("unsupported_service"))?;
    validate_service_start_readiness(state, service)
        .map_err(|err| anyhow::anyhow!(err.error_code))?;
    let workspace = state.skill_rt.workspace_root.to_string_lossy();
    let log_file = format!("logs/{}.log", service);
    let cmd = format!(
        "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
        shell_escape_arg(workspace.as_ref()),
        script_name,
        shell_escape_arg(profile.as_str()),
        shell_escape_arg(log_file.as_str())
    );
    spawn_background_shell(&cmd)?;
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    if !service_is_running(service) {
        anyhow::bail!(
            "service did not enter running state: {service}. check logs/{service}.log and channel config"
        );
    }
    Ok(())
}

async fn maybe_complete_feishu_official_scan(
    state: &AppState,
    session: PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    if !matches!(session.status.as_str(), "pending" | "detected") {
        return Ok(session);
    }
    let Some(device_code) = session
        .install_device_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(session);
    };

    let poll = poll_feishu_official_registration(state, device_code).await?;
    if let (Some(client_id), Some(client_secret), Some(_open_id)) = (
        poll.client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        poll.client_secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        poll.user_info
            .as_ref()
            .and_then(|user| user.open_id.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        write_feishu_generated_credentials(state, client_id, client_secret)?;
        if let Err(err) = start_service_if_needed(state, "feishud").await {
            let mut db = state
                .core
                .db
                .get()
                .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
            return mark_pending_channel_bind_session_failed(&mut db, session.id, &err.to_string());
        }
        return Ok(session);
    }

    let Some(error_code) = poll
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(session);
    };
    let error_text = poll
        .error_description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|detail| format!("{error_code}: {detail}"))
        .unwrap_or_else(|| error_code.to_string());
    let mut db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    match error_code {
        "authorization_pending" | "slow_down" => Ok(session),
        "expired_token" => mark_pending_channel_bind_session_expired(&mut db, session.id),
        "access_denied" => {
            mark_pending_channel_bind_session_failed(&mut db, session.id, &error_text)
        }
        _ => mark_pending_channel_bind_session_failed(&mut db, session.id, &error_text),
    }
}

fn find_detectable_feishu_bind_session(
    db: &rusqlite::Connection,
    bind_token: Option<&str>,
) -> anyhow::Result<Option<PendingChannelBindSession>> {
    let Some(bind_token) = bind_token.map(str::trim).filter(|token| !token.is_empty()) else {
        return Ok(None);
    };
    get_pending_channel_bind_session_by_token(db, bind_token)
}

async fn start_feishu_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StartFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can start feishu binds".to_string()),
            }),
        );
    }

    let ttl_seconds = clamp_feishu_bind_ttl_seconds(req.expires_in_seconds);
    let default_expires_at = current_unix_ts()
        .saturating_add(ttl_seconds as i64)
        .to_string();
    let session = {
        let mut db = match state.core.db.get() {
            Ok(db) => db,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("db lock poisoned".to_string()),
                    }),
                );
            }
        };
        match create_pending_channel_bind_session(
            &mut db,
            "feishu",
            &identity.user_key,
            &default_expires_at,
        ) {
            Ok(session) => session,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("create feishu bind session failed: {err}")),
                    }),
                );
            }
        }
    };

    let config = match load_feishu_config_response(&state, Some(&identity.user_key)) {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read feishu config failed: {err}")),
                }),
            );
        }
    };
    if config.bind_ready {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(feishu_bind_session_response(&state, session)),
                error: None,
            }),
        );
    }

    let begin = match begin_feishu_official_registration(&state).await {
        Ok(begin) => begin,
        Err(err) => {
            let mut db = match state.core.db.get() {
                Ok(db) => db,
                Err(_) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("db lock poisoned".to_string()),
                        }),
                    );
                }
            };
            let _ = mark_pending_channel_bind_session_failed(&mut db, session.id, &err.to_string());
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("start feishu official registration failed: {err}")),
                }),
            );
        }
    };
    let begin_expire_seconds = begin.expire_in.unwrap_or(ttl_seconds);
    let session_expires_at = current_unix_ts()
        .saturating_add(begin_expire_seconds.min(ttl_seconds) as i64)
        .to_string();
    let mut db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    match attach_pending_channel_bind_session_install_flow(
        &mut db,
        session.id,
        &begin.device_code,
        &begin.verification_uri_complete,
        begin.interval.unwrap_or(5) as i64,
        &session_expires_at,
    ) {
        Ok(session) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(feishu_bind_session_response(&state, session)),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "persist feishu official registration failed: {err}"
                )),
            }),
        ),
    }
}

async fn get_feishu_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can inspect feishu binds".to_string()),
            }),
        );
    }

    let session = {
        let mut db = match state.core.db.get() {
            Ok(db) => db,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("db lock poisoned".to_string()),
                    }),
                );
            }
        };
        match get_pending_channel_bind_session_by_id(&db, session_id) {
            Ok(Some(session)) => {
                if session.user_key != identity.user_key {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("feishu bind session not found".to_string()),
                        }),
                    );
                }
                match maybe_expire_feishu_bind_session(&mut db, session) {
                    Ok(session) => session,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                ok: false,
                                data: None,
                                error: Some(format!("refresh feishu bind session failed: {err}")),
                            }),
                        );
                    }
                }
            }
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("feishu bind session not found".to_string()),
                    }),
                );
            }
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("get feishu bind session failed: {err}")),
                    }),
                );
            }
        }
    };

    match maybe_complete_feishu_official_scan(&state, session).await {
        Ok(session) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(feishu_bind_session_response(&state, session)),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("refresh feishu bind session failed: {err}")),
            }),
        ),
    }
}

async fn detect_feishu_bind_session_handler(
    State(state): State<AppState>,
    Json(req): Json<DetectFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<DetectFeishuBindSessionResponse>>,
) {
    let external_user_id = req.external_user_id.trim();
    let external_chat_id = req.external_chat_id.trim();
    let bind_token = req
        .bind_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty());
    if external_user_id.is_empty() || external_chat_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("external_user_id and external_chat_id are required".to_string()),
            }),
        );
    }
    if bind_token.is_none() {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(DetectFeishuBindSessionResponse {
                    matched: false,
                    session: None,
                }),
                error: None,
            }),
        );
    }

    let mut db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    let Some(session) = (match find_detectable_feishu_bind_session(&db, bind_token) {
        Ok(session) => session,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("load feishu bind session failed: {err}")),
                }),
            );
        }
    }) else {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(DetectFeishuBindSessionResponse {
                    matched: false,
                    session: None,
                }),
                error: None,
            }),
        );
    };

    let session = match maybe_expire_feishu_bind_session(&mut db, session) {
        Ok(session) => session,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("refresh feishu bind session failed: {err}")),
                }),
            );
        }
    };
    if session.status == "expired" {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(DetectFeishuBindSessionResponse {
                    matched: false,
                    session: Some(feishu_bind_session_response(&state, session)),
                }),
                error: None,
            }),
        );
    }

    let session = if session.status == "bound" {
        session
    } else {
        let detected = match mark_pending_channel_bind_session_detected(
            &mut db,
            session.id,
            external_user_id,
            external_chat_id,
        ) {
            Ok(session) => session,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("detect feishu bind session failed: {err}")),
                    }),
                );
            }
        };
        match finalize_pending_channel_bind_session(&mut db, detected.id) {
            Ok(session) => session,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("finalize feishu bind session failed: {err}")),
                    }),
                );
            }
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(DetectFeishuBindSessionResponse {
                matched: true,
                session: Some(feishu_bind_session_response(&state, session)),
            }),
            error: None,
        }),
    )
}

async fn update_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
    Json(req): Json<UpdateAuthKeyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can update auth keys".to_string()),
            }),
        );
    }

    let role = req.role.as_deref();
    let role = role.map(str::trim).filter(|v| !v.is_empty());
    match update_auth_key_by_id(&state, key_id, role, req.enabled, &identity.user_key) {
        Ok(true) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "updated": true })),
                error: None,
            }),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("update auth key failed: {err}")),
            }),
        ),
    }
}

async fn delete_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can delete auth keys".to_string()),
            }),
        );
    }

    match delete_auth_key_by_id(&state, key_id, &identity.user_key) {
        Ok(true) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "deleted": true })),
                error: None,
            }),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("delete auth key failed: {err}")),
            }),
        ),
    }
}

async fn create_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAuthKeyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can create auth keys".to_string()),
            }),
        );
    }
    match create_auth_key(&state, req.role.as_str()) {
        Ok(user_key) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("create auth key failed: {err}")),
            }),
        ),
    }
}

fn ui_auth_error(message: &str) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(message.to_string()),
        }),
    )
}

pub(crate) fn require_ui_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<Value>>)> {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(ui_auth_error("Missing X-RustClaw-Key header"));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(ui_auth_error("Invalid key")),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("auth lookup failed: {err}")),
            }),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct WebdInternalVerifyRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct AdminWebdAccountRequest {
    username: String,
    password: String,
    #[serde(default)]
    key_id: Option<i64>,
    #[serde(default)]
    user_key: String,
}

async fn webd_internal_verify_login(
    State(state): State<AppState>,
    Json(req): Json<WebdInternalVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let db = match state.core.db.get() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    match verify_webd_password_login(&db, &req.username, &req.password) {
        Ok(Some(user_key)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("invalid username or password".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("login failed: {err}")),
            }),
        ),
    }
}

async fn admin_upsert_webd_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AdminWebdAccountRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(id) => id,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can manage webd accounts".to_string()),
            }),
        );
    }
    let target_user_key = if let Some(key_id) = req.key_id {
        match get_auth_key_value_by_id(&state, key_id) {
            Ok(Some(user_key)) => user_key,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("auth key not found".to_string()),
                    }),
                );
            }
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("load auth key failed: {err}")),
                    }),
                );
            }
        }
    } else {
        let user_key = req.user_key.trim().to_string();
        if user_key.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("key_id or user_key is required".to_string()),
                }),
            );
        }
        user_key
    };
    let db = match state.core.db.get() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("db lock poisoned".to_string()),
                }),
            );
        }
    };
    match upsert_webd_login_account(&db, &req.username, &req.password, &target_user_key) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "updated": true })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err.to_string()),
            }),
        ),
    }
}

async fn verify_ui_key(
    State(state): State<AppState>,
    Json(req): Json<UiKeyVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match resolve_auth_identity_by_key(&state, &req.user_key) {
        Ok(Some(identity)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid key".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("auth lookup failed: {err}")),
            }),
        ),
    }
}

async fn auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match require_ui_identity(&state, &headers) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Err((status, Json(resp))) => (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        ),
    }
}

async fn resolve_channel_binding(
    State(state): State<AppState>,
    Json(req): Json<ResolveChannelBindingRequest>,
) -> (StatusCode, Json<ApiResponse<ResolveChannelBindingResponse>>) {
    match resolve_channel_binding_identity(
        &state,
        &scoped_channel_name(req.channel, req.telegram_bot_name.as_deref()),
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
    ) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(ResolveChannelBindingResponse {
                    bound: identity.is_some(),
                    identity,
                }),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("resolve channel binding failed: {err}")),
            }),
        ),
    }
}

async fn bind_channel_key(
    State(state): State<AppState>,
    Json(req): Json<BindChannelKeyRequest>,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match bind_channel_identity(
        &state,
        &scoped_channel_name(req.channel, req.telegram_bot_name.as_deref()),
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
        &req.user_key,
    ) {
        Ok(Some(identity)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid key".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("bind channel key failed: {err}")),
            }),
        ),
    }
}
