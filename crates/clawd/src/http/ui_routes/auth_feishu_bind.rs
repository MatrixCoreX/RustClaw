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

fn clamp_channel_bind_ttl_seconds(raw: Option<u64>) -> u64 {
    raw.unwrap_or(FEISHU_BIND_SESSION_DEFAULT_TTL_SECONDS)
        .clamp(
            FEISHU_BIND_SESSION_MIN_TTL_SECONDS,
            FEISHU_BIND_SESSION_MAX_TTL_SECONDS,
        )
}

#[derive(Debug, Deserialize, Default)]
struct OfficialRegistrationInitResponse {
    #[serde(default)]
    supported_auth_methods: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct OfficialRegistrationBeginResponse {
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
struct OfficialRegistrationUserInfo {
    #[serde(default)]
    open_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct OfficialRegistrationPollResponse {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    user_info: Option<OfficialRegistrationUserInfo>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentAppChannel {
    Feishu,
    Lark,
}

impl AgentAppChannel {
    fn channel(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
            Self::Lark => "lark",
        }
    }

    fn service(self) -> &'static str {
        match self {
            Self::Feishu => "feishud",
            Self::Lark => "larkd",
        }
    }

    fn accounts_env(self) -> &'static str {
        match self {
            Self::Feishu => "FEISHU_ACCOUNTS_BASE_URL",
            Self::Lark => "LARK_ACCOUNTS_BASE_URL",
        }
    }

    fn default_accounts_base_url(self) -> &'static str {
        match self {
            Self::Feishu => FEISHU_OFFICIAL_ACCOUNTS_BASE_URL,
            Self::Lark => LARK_OFFICIAL_ACCOUNTS_BASE_URL,
        }
    }

    fn app_link_base_url(self) -> &'static str {
        match self {
            Self::Feishu => "https://applink.feishu.cn",
            Self::Lark => "https://applink.larksuite.com",
        }
    }
}

fn official_accounts_base_url(platform: AgentAppChannel) -> String {
    claw_core::product_identity::env_string(platform.accounts_env())
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| platform.default_accounts_base_url().to_string())
}

async fn call_official_registration<T: DeserializeOwned>(
    state: &AppState,
    platform: AgentAppChannel,
    params: &[(&str, &str)],
) -> anyhow::Result<T> {
    let url = format!(
        "{}/oauth/v1/app/registration",
        official_accounts_base_url(platform)
    );
    let resp = state.core.http_client.post(url).form(params).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    serde_json::from_str::<T>(&body).map_err(|err| {
        anyhow::anyhow!(
            "decode {} registration response failed: status={} body={} err={}",
            platform.channel(),
            status,
            body,
            err
        )
    })
}

async fn begin_official_registration(
    state: &AppState,
    platform: AgentAppChannel,
) -> anyhow::Result<OfficialRegistrationBeginResponse> {
    let init = call_official_registration::<OfficialRegistrationInitResponse>(
        state, platform, &[("action", "init")],
    ).await?;
    if !init
        .supported_auth_methods
        .iter()
        .any(|method| method == "client_secret")
    {
        anyhow::bail!(
            "{} registration does not support client_secret auth",
            platform.channel()
        );
    }
    let begin = call_official_registration::<OfficialRegistrationBeginResponse>(
        state,
        platform,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;
    if begin.device_code.trim().is_empty() || begin.verification_uri_complete.trim().is_empty() {
        anyhow::bail!(
            "{} registration did not return a device_code or verification url",
            platform.channel()
        );
    }
    Ok(begin)
}

async fn poll_official_registration(
    state: &AppState,
    platform: AgentAppChannel,
    device_code: &str,
) -> anyhow::Result<OfficialRegistrationPollResponse> {
    call_official_registration::<OfficialRegistrationPollResponse>(
        state,
        platform,
        &[("action", "poll"), ("device_code", device_code)],
    )
    .await
}

fn agent_app_entry_url_for_app_id(platform: AgentAppChannel, app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "{}/client/bot/open?appId={trimmed}",
        platform.app_link_base_url()
    ))
}

fn channel_bind_entry_url(
    state: &AppState,
    platform: AgentAppChannel,
    session: Option<&PendingChannelBindSession>,
) -> Option<String> {
    let configured_app_id = match platform {
        AgentAppChannel::Feishu => load_feishu_config_response(state, None)
            .ok()
            .filter(|config| config.bind_ready)
            .map(|config| config.app_id),
        AgentAppChannel::Lark => load_lark_config_response(state, None)
            .ok()
            .filter(|config| config.bind_ready)
            .map(|config| config.app_id),
    };
    if let Some(app_id) = configured_app_id {
        if let Some(entry_url) = agent_app_entry_url_for_app_id(platform, &app_id) {
            return Some(entry_url);
        }
    }
    session
        .and_then(|session| session.install_verification_url.clone())
        .filter(|url| !url.trim().is_empty())
}

fn channel_bind_session_response(
    state: &AppState,
    platform: AgentAppChannel,
    session: PendingChannelBindSession,
) -> FeishuBindSessionStatusResponse {
    let entry_url = channel_bind_entry_url(state, platform, Some(&session));
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
        poll_interval_seconds: session
            .install_poll_interval_seconds
            .and_then(|seconds| u64::try_from(seconds).ok()),
    }
}

fn maybe_expire_channel_bind_session(
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

fn write_generated_credentials(
    state: &AppState,
    platform: AgentAppChannel,
    app_id: &str,
    app_secret: &str,
) -> anyhow::Result<()> {
    let (relative_path, output) = match platform {
        AgentAppChannel::Feishu => {
            let raw = read_feishu_config_raw(state)?;
            (
                "configs/channels/feishu.toml",
                update_feishu_config_raw_preserving_format(&raw, app_id, app_secret),
            )
        }
        AgentAppChannel::Lark => {
            let raw = read_lark_config_raw(state)?;
            (
                "configs/channels/lark.toml",
                update_lark_config_raw_preserving_format(&raw, app_id, app_secret),
            )
        }
    };
    write_workspace_and_mounted_file(
        &state.skill_rt.workspace_root,
        relative_path,
        &output,
    )?;
    Ok(())
}

async fn start_service_if_needed(state: &AppState, service: &str) -> anyhow::Result<()> {
    if service_is_running(service) {
        return Ok(());
    }
    let profile = claw_core::product_identity::env_string("START_PROFILE")
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

async fn maybe_complete_official_scan(
    state: &AppState,
    platform: AgentAppChannel,
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

    let poll = poll_official_registration(state, platform, device_code).await?;
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
        write_generated_credentials(state, platform, client_id, client_secret)?;
        if let Err(err) = start_service_if_needed(state, platform.service()).await {
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

fn find_detectable_channel_bind_session(
    db: &rusqlite::Connection,
    platform: AgentAppChannel,
    bind_token: Option<&str>,
) -> anyhow::Result<Option<PendingChannelBindSession>> {
    let Some(bind_token) = bind_token.map(str::trim).filter(|token| !token.is_empty()) else {
        return Ok(None);
    };
    Ok(get_pending_channel_bind_session_by_token(db, bind_token)?
        .filter(|session| session.channel == platform.channel()))
}

async fn start_channel_bind_session(
    state: AppState,
    headers: HeaderMap,
    req: StartFeishuBindSessionRequest,
    platform: AgentAppChannel,
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
                error: Some(format!(
                    "only admin can start {} binds",
                    platform.channel()
                )),
            }),
        );
    }

    let ttl_seconds = clamp_channel_bind_ttl_seconds(req.expires_in_seconds);
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
            platform.channel(),
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
                        error: Some(format!(
                            "create {} bind session failed: {err}",
                            platform.channel()
                        )),
                    }),
                );
            }
        }
    };

    let bind_ready = match platform {
        AgentAppChannel::Feishu => load_feishu_config_response(&state, Some(&identity.user_key))
            .map(|config| config.bind_ready),
        AgentAppChannel::Lark => load_lark_config_response(&state, Some(&identity.user_key))
            .map(|config| config.bind_ready),
    };
    let bind_ready = match bind_ready {
        Ok(bind_ready) => bind_ready,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "read {} config failed: {err}",
                        platform.channel()
                    )),
                }),
            );
        }
    };
    if bind_ready {
        return (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(channel_bind_session_response(&state, platform, session)),
                error: None,
            }),
        );
    }

    let begin = match begin_official_registration(&state, platform).await {
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
                    error: Some(format!(
                        "start {} official registration failed: {err}",
                        platform.channel()
                    )),
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
                data: Some(channel_bind_session_response(&state, platform, session)),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "persist {} official registration failed: {err}",
                    platform.channel()
                )),
            }),
        ),
    }
}

async fn get_channel_bind_session(
    state: AppState,
    headers: HeaderMap,
    session_id: i64,
    platform: AgentAppChannel,
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
                error: Some(format!(
                    "only admin can inspect {} binds",
                    platform.channel()
                )),
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
                if session.user_key != identity.user_key || session.channel != platform.channel() {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!(
                                "{} bind session not found",
                                platform.channel()
                            )),
                        }),
                    );
                }
                match maybe_expire_channel_bind_session(&mut db, session) {
                    Ok(session) => session,
                    Err(err) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse {
                                ok: false,
                                data: None,
                                error: Some(format!(
                                    "refresh {} bind session failed: {err}",
                                    platform.channel()
                                )),
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
                        error: Some(format!(
                            "{} bind session not found",
                            platform.channel()
                        )),
                    }),
                );
            }
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "get {} bind session failed: {err}",
                            platform.channel()
                        )),
                    }),
                );
            }
        }
    };

    match maybe_complete_official_scan(&state, platform, session).await {
        Ok(session) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(channel_bind_session_response(&state, platform, session)),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "refresh {} bind session failed: {err}",
                    platform.channel()
                )),
            }),
        ),
    }
}

async fn detect_channel_bind_session(
    state: AppState,
    req: DetectFeishuBindSessionRequest,
    platform: AgentAppChannel,
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
                    identity: None,
                    pending_resume: None,
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
    let Some(session) = (match find_detectable_channel_bind_session(&db, platform, bind_token) {
        Ok(session) => session,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "load {} bind session failed: {err}",
                        platform.channel()
                    )),
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
                    identity: None,
                    pending_resume: None,
                }),
                error: None,
            }),
        );
    };

    let session = match maybe_expire_channel_bind_session(&mut db, session) {
        Ok(session) => session,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "refresh {} bind session failed: {err}",
                        platform.channel()
                    )),
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
                    session: Some(channel_bind_session_response(&state, platform, session)),
                    identity: None,
                    pending_resume: None,
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
                        error: Some(format!(
                            "detect {} bind session failed: {err}",
                            platform.channel()
                        )),
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
                        error: Some(format!(
                            "finalize {} bind session failed: {err}",
                            platform.channel()
                        )),
                    }),
                );
            }
        }
    };

    let resume_user_key = session.user_key.clone();
    let session_response = channel_bind_session_response(&state, platform, session);
    drop(db);
    let identity = resolve_auth_identity_by_key(&state, &resume_user_key)
        .ok()
        .flatten();
    let pending_resume = match identity.as_ref() {
        Some(identity) => resume_pending_channel_request_after_bind(
            &state,
            platform.channel(),
            Some(external_user_id),
            Some(external_chat_id),
            &identity,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                "pending request resume after {} session bind failed: {error}",
                platform.channel()
            );
            None
        }),
        _ => None,
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(DetectFeishuBindSessionResponse {
                matched: true,
                session: Some(session_response),
                identity,
                pending_resume,
            }),
            error: None,
        }),
    )
}

async fn start_feishu_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StartFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    start_channel_bind_session(state, headers, req, AgentAppChannel::Feishu).await
}

async fn start_lark_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StartFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    start_channel_bind_session(state, headers, req, AgentAppChannel::Lark).await
}

async fn get_feishu_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    get_channel_bind_session(state, headers, session_id, AgentAppChannel::Feishu).await
}

async fn get_lark_bind_session_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<i64>,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    get_channel_bind_session(state, headers, session_id, AgentAppChannel::Lark).await
}

async fn detect_feishu_bind_session_handler(
    State(state): State<AppState>,
    Json(req): Json<DetectFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<DetectFeishuBindSessionResponse>>,
) {
    detect_channel_bind_session(state, req, AgentAppChannel::Feishu).await
}

async fn detect_lark_bind_session_handler(
    State(state): State<AppState>,
    Json(req): Json<DetectFeishuBindSessionRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<DetectFeishuBindSessionResponse>>,
) {
    detect_channel_bind_session(state, req, AgentAppChannel::Lark).await
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

fn ui_auth_error(error_code: &'static str) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "owner_layer": "ui_auth",
                "error_code": error_code,
                "status_code": error_code,
                "message_key": format!("clawd.ui.auth.{error_code}"),
            })),
            error: Some(error_code.to_string()),
        }),
    )
}

fn ui_auth_code_error<T>(error_code: &'static str) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(error_code.to_string()),
        }),
    )
}

pub(crate) fn require_ui_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<Value>>)> {
    let Some(raw_key) = crate::auth_key_from_headers(headers)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(ui_auth_error("auth_key_required"));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(ui_auth_error("auth_key_invalid")),
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

pub(crate) fn require_ui_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<Value>>)> {
    let identity = require_ui_identity(state, headers)?;
    if !identity.role.eq_ignore_ascii_case("admin") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: Some(json!({
                    "error_code": "admin_required",
                    "message_key": "clawd.ui.auth.admin_required",
                })),
                error: Some("admin_required".to_string()),
            }),
        ));
    }
    Ok(identity)
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
    let verified = {
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
        verify_webd_password_login(&db, &req.username, &req.password)
    };
    match verified {
        Ok(Some(user_key)) => match resolve_auth_identity_by_key(&state, &user_key) {
            Ok(Some(identity)) => (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "user_key": user_key,
                        "role": identity.role,
                        "principal_id": identity.principal_id,
                    })),
                    error: None,
                }),
            ),
            Ok(None) => (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("invalid_credentials".to_string()),
                }),
            ),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("identity resolution failed: {err}")),
                }),
            ),
        },
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: Some(json!({
                    "owner_layer": "webd_login",
                    "error_code": "invalid_credentials",
                    "status_code": "invalid_credentials",
                    "message_key": "clawd.webd.login.invalid_credentials",
                })),
                error: Some("invalid_credentials".to_string()),
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
        Ok(None) => ui_auth_code_error("auth_key_invalid"),
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

async fn store_pending_channel_request_handler(
    State(state): State<AppState>,
    Json(req): Json<PendingChannelRequestStoreRequest>,
) -> (StatusCode, Json<ApiResponse<PendingChannelRequestStatus>>) {
    match store_pending_channel_request(&state, &req) {
        Ok(status) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(status),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(error.to_string()),
            }),
        ),
    }
}

fn pending_attachment_revalidation_error(
    state: &AppState,
    request: &claw_core::types::SubmitTaskRequest,
) -> Option<&'static str> {
    let Ok(workspace_root) = std::fs::canonicalize(&state.skill_rt.workspace_root) else {
        return Some("pending_request_attachment_invalid");
    };
    for raw_path in request_attachment_paths(request) {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Some("pending_request_attachment_missing");
        }
        let path = std::path::Path::new(trimmed);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            state.skill_rt.workspace_root.join(path)
        };
        let Ok(canonical) = std::fs::canonicalize(candidate) else {
            return Some("pending_request_attachment_missing");
        };
        if !canonical.starts_with(&workspace_root) || !canonical.is_file() {
            return Some("pending_request_attachment_invalid");
        }
    }
    if let Some(ingress) = request.ingress.as_ref() {
        for attachment in &ingress.attachments {
            let Some(expected_size) = attachment.size else {
                continue;
            };
            let path = std::path::Path::new(attachment.path.trim());
            let candidate = if path.is_absolute() {
                path.to_path_buf()
            } else {
                state.skill_rt.workspace_root.join(path)
            };
            let Ok(actual_size) = std::fs::metadata(candidate).map(|metadata| metadata.len()) else {
                return Some("pending_request_attachment_missing");
            };
            if actual_size != expected_size {
                return Some("pending_request_attachment_changed");
            }
        }
    }
    None
}

async fn resume_pending_channel_request_after_bind(
    state: &AppState,
    binding_channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    identity: &AuthIdentity,
) -> anyhow::Result<Option<PendingChannelRequestStatus>> {
    let Some(candidate) = pending_channel_resume_candidate(
        state,
        binding_channel,
        external_user_id,
        external_chat_id,
    )? else {
        return Ok(None);
    };
    let Some(mut request) = candidate.request else {
        return Ok(Some(candidate.status));
    };
    if let Some(error_code) = pending_attachment_revalidation_error(state, &request) {
        return finish_pending_channel_resume(
            state,
            candidate.status.pending_request_id,
            None,
            Some(error_code),
        )
        .map(Some);
    }
    request.user_id = Some(identity.user_id);
    request.chat_id = Some(identity.chat_id);
    request.user_key = Some(identity.user_key.clone());
    let (status, Json(response)) = crate::submit_task(
        State(state.clone()),
        HeaderMap::new(),
        Json(request),
    )
    .await;
    if status.is_success() && response.ok {
        let task_id = response.data.map(|data: SubmitTaskResponse| data.task_id);
        return finish_pending_channel_resume(
            state,
            candidate.status.pending_request_id,
            task_id,
            task_id.is_none().then_some("pending_request_submit_missing_task"),
        )
        .map(Some);
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return finish_pending_channel_resume(
            state,
            candidate.status.pending_request_id,
            None,
            Some("pending_request_permission_changed"),
        )
        .map(Some);
    }
    let mut retryable = candidate.status;
    retryable.error_code = Some("pending_request_resume_retryable".to_string());
    Ok(Some(retryable))
}

async fn bind_channel_key(
    State(state): State<AppState>,
    Json(req): Json<BindChannelKeyRequest>,
) -> (StatusCode, Json<ApiResponse<BindChannelKeyResponse>>) {
    let binding_channel = scoped_channel_name(req.channel, req.telegram_bot_name.as_deref());
    match bind_channel_identity(
        &state,
        &binding_channel,
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
        &req.user_key,
    ) {
        Ok(Some(identity)) => match resume_pending_channel_request_after_bind(
            &state,
            &binding_channel,
            req.external_user_id.as_deref(),
            req.external_chat_id.as_deref(),
            &identity,
        )
        .await
        {
            Ok(pending_resume) => (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(BindChannelKeyResponse {
                        identity,
                        pending_resume,
                    }),
                    error: None,
                }),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("pending channel resume failed: {error}")),
                }),
            ),
        },
        Ok(None) => ui_auth_code_error("auth_key_invalid"),
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

#[cfg(test)]
#[path = "auth_feishu_bind_tests.rs"]
mod pending_request_tests;
