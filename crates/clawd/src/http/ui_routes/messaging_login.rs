async fn whatsapp_web_login_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let base = state
        .channels
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.not_configured".to_string()),
            }),
        );
    }
    let url = format!("{base}/v1/login-status");
    let resp = match state.core.http_client.get(&url).send().await {
        Ok(v) => v,
        Err(_err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("whatsapp_web.login_status_unavailable".to_string()),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.login_status_unavailable".to_string()),
            }),
        );
    }
    let data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(_err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("whatsapp_web.login_status_invalid".to_string()),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

const WECHAT_UI_LOGIN_SESSION_TTL_SECONDS: u64 = 10 * 60;

#[derive(Clone, Debug)]
struct WechatUiLoginSession {
    principal_id: String,
    provider_session_key: Option<String>,
    expires_at_epoch: u64,
}

#[derive(Default)]
struct WechatUiLoginSessionStore {
    sessions: HashMap<String, WechatUiLoginSession>,
}

impl WechatUiLoginSessionStore {
    fn prune_expired(&mut self, now: u64) {
        self.sessions
            .retain(|_, session| session.expires_at_epoch > now);
    }

    fn reserve(&mut self, principal_id: &str, now: u64) -> Result<String, &'static str> {
        self.prune_expired(now);
        if let Some((token, session)) = self
            .sessions
            .iter_mut()
            .find(|(_, session)| session.principal_id == principal_id)
        {
            session.provider_session_key = None;
            session.expires_at_epoch = now.saturating_add(WECHAT_UI_LOGIN_SESSION_TTL_SECONDS);
            return Ok(token.clone());
        }
        if !self.sessions.is_empty() {
            return Err("wechat.login_session_in_use");
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions.insert(
            token.clone(),
            WechatUiLoginSession {
                principal_id: principal_id.to_string(),
                provider_session_key: None,
                expires_at_epoch: now.saturating_add(WECHAT_UI_LOGIN_SESSION_TTL_SECONDS),
            },
        );
        Ok(token)
    }

    fn attach_provider_session(
        &mut self,
        token: &str,
        principal_id: &str,
        provider_session_key: String,
        now: u64,
    ) -> Result<(), &'static str> {
        self.prune_expired(now);
        let Some(session) = self.sessions.get_mut(token) else {
            return Err("wechat.login_session_expired");
        };
        if session.principal_id != principal_id {
            return Err("wechat.login_session_owner_mismatch");
        }
        session.provider_session_key = Some(provider_session_key);
        session.expires_at_epoch = now.saturating_add(WECHAT_UI_LOGIN_SESSION_TTL_SECONDS);
        Ok(())
    }

    fn resolve(
        &mut self,
        token: &str,
        principal_id: &str,
        now: u64,
    ) -> Result<WechatUiLoginSession, &'static str> {
        self.prune_expired(now);
        let Some(session) = self.sessions.get(token) else {
            return Err("wechat.login_session_expired");
        };
        if session.principal_id != principal_id {
            return Err("wechat.login_session_owner_mismatch");
        }
        if session.provider_session_key.is_none() {
            return Err("wechat.login_session_not_ready");
        }
        Ok(session.clone())
    }

    fn client_token_for_provider(
        &mut self,
        principal_id: &str,
        provider_session_key: &str,
        now: u64,
    ) -> Option<String> {
        self.prune_expired(now);
        self.sessions.iter().find_map(|(token, session)| {
            (session.principal_id == principal_id
                && session.provider_session_key.as_deref() == Some(provider_session_key))
            .then(|| token.clone())
        })
    }

    fn remove(&mut self, token: &str) {
        self.sessions.remove(token);
    }
}

static WECHAT_UI_LOGIN_SESSIONS: OnceLock<Arc<Mutex<WechatUiLoginSessionStore>>> = OnceLock::new();

fn wechat_ui_login_sessions() -> &'static Arc<Mutex<WechatUiLoginSessionStore>> {
    WECHAT_UI_LOGIN_SESSIONS
        .get_or_init(|| Arc::new(Mutex::new(WechatUiLoginSessionStore::default())))
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn wechat_login_error(
    status: StatusCode,
    error_code: &str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "error_code": error_code,
                "message_key": format!("clawd.ui.{error_code}"),
            })),
            error: Some(error_code.to_string()),
        }),
    )
}

fn wechat_external_user_id(data: &Value) -> Option<&str> {
    data.get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn project_wechat_login_status(
    mut data: Value,
    current_user_bound: bool,
    client_session_key: Option<&str>,
) -> Result<Value, &'static str> {
    let provider_connected = data
        .get("connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(object) = data.as_object_mut() else {
        return Err("wechat.login_status_invalid");
    };
    object.remove("user_id");
    object.insert(
        "provider_connected".to_string(),
        Value::Bool(provider_connected),
    );
    object.insert(
        "current_user_bound".to_string(),
        Value::Bool(current_user_bound),
    );
    object.insert(
        "connected".to_string(),
        Value::Bool(provider_connected && current_user_bound),
    );
    let binding_status = match (provider_connected, current_user_bound) {
        (true, true) => "bound",
        (true, false) => "connected_unbound",
        (false, true) => "bound_offline",
        (false, false) => "unbound",
    };
    object.insert(
        "binding_status".to_string(),
        Value::String(binding_status.to_string()),
    );
    if let Some(client_session_key) = client_session_key {
        object.insert(
            "session_key".to_string(),
            Value::String(client_session_key.to_string()),
        );
    } else {
        object.remove("session_key");
        object.remove("qrcode_url");
        object.remove("message");
        object.insert("qr_ready".to_string(), Value::Bool(false));
    }
    Ok(data)
}

#[derive(Debug, Deserialize, Default)]
struct WechatQrStartRequest {
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct WechatQrWaitRequest {
    session_key: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

fn wechatd_base_url(state: &AppState) -> Result<String, (StatusCode, Json<ApiResponse<Value>>)> {
    let config = load_wechat_config_response(state).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read wechat config failed: {err}")),
            }),
        )
    })?;
    let listen = config.listen.trim();
    if !config.enabled || listen.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("wechat daemon is not configured".to_string()),
            }),
        ));
    }
    let host_port = if let Some(rest) = listen.strip_prefix("0.0.0.0:") {
        format!("127.0.0.1:{rest}")
    } else if let Some(rest) = listen.strip_prefix("[::]:") {
        format!("127.0.0.1:{rest}")
    } else {
        listen.to_string()
    };
    Ok(format!("http://{host_port}"))
}

async fn fetch_wechatd_login_status(
    state: &AppState,
    base: &str,
) -> Result<Value, (StatusCode, Json<ApiResponse<Value>>)> {
    let url = format!("{}/login/status", base.trim_end_matches('/'));
    let resp = state
        .core
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|_| {
            wechat_login_error(
                StatusCode::BAD_GATEWAY,
                "wechat.login_status_unavailable",
            )
        })?;
    if !resp.status().is_success() {
        return Err(wechat_login_error(
            StatusCode::BAD_GATEWAY,
            "wechat.login_status_unavailable",
        ));
    }
    resp.json::<Value>().await.map_err(|_| {
        wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.login_status_invalid")
    })
}

fn current_user_owns_wechat_binding(
    state: &AppState,
    identity: &AuthIdentity,
    data: &Value,
) -> Result<bool, (StatusCode, Json<ApiResponse<Value>>)> {
    let Some(external_user_id) = wechat_external_user_id(data) else {
        return Ok(false);
    };
    resolve_channel_binding_identity(
        state,
        "wechat",
        Some(external_user_id),
        Some(external_user_id),
    )
    .map(|owner| {
        owner
            .as_ref()
            .is_some_and(|owner| owner.principal_id == identity.principal_id)
    })
    .map_err(|_| {
        wechat_login_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "wechat.binding_status_failed",
        )
    })
}

fn wechat_client_session_for_status(
    identity: &AuthIdentity,
    data: &Value,
) -> Result<Option<String>, (StatusCode, Json<ApiResponse<Value>>)> {
    let Some(provider_session_key) = data
        .get("session_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let mut sessions = wechat_ui_login_sessions().lock().map_err(|_| {
        wechat_login_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "wechat.login_session_store_unavailable",
        )
    })?;
    Ok(sessions.client_token_for_provider(
        &identity.principal_id,
        provider_session_key,
        current_epoch_seconds(),
    ))
}

async fn wechat_login_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let base = match wechatd_base_url(&state) {
        Ok(base) => base,
        Err(resp) => return resp,
    };
    let data = match fetch_wechatd_login_status(&state, &base).await {
        Ok(data) => data,
        Err(resp) => return resp,
    };
    let current_user_bound = match current_user_owns_wechat_binding(&state, &identity, &data) {
        Ok(bound) => bound,
        Err(resp) => return resp,
    };
    let client_session_key = match wechat_client_session_for_status(&identity, &data) {
        Ok(session_key) => session_key,
        Err(resp) => return resp,
    };
    let data = match project_wechat_login_status(
        data,
        current_user_bound,
        client_session_key.as_deref(),
    ) {
        Ok(data) => data,
        Err(error_code) => return wechat_login_error(StatusCode::BAD_GATEWAY, error_code),
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn wechat_login_qr_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WechatQrStartRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let base = match wechatd_base_url(&state) {
        Ok(base) => base,
        Err(resp) => return resp,
    };
    let client_session_key = {
        let mut sessions = match wechat_ui_login_sessions().lock() {
            Ok(sessions) => sessions,
            Err(_) => {
                return wechat_login_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "wechat.login_session_store_unavailable",
                )
            }
        };
        match sessions.reserve(&identity.principal_id, current_epoch_seconds()) {
            Ok(session_key) => session_key,
            Err(error_code) => return wechat_login_error(StatusCode::CONFLICT, error_code),
        }
    };
    let url = format!("{}/login/qr/start", base.trim_end_matches('/'));
    let resp = match state
        .core
        .http_client
        .post(&url)
        .json(&json!({ "force": req.force }))
        .send()
        .await
    {
        Ok(v) => v,
        Err(_) => {
            if let Ok(mut sessions) = wechat_ui_login_sessions().lock() {
                sessions.remove(&client_session_key);
            }
            return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_start_unavailable");
        }
    };
    if !resp.status().is_success() {
        if let Ok(mut sessions) = wechat_ui_login_sessions().lock() {
            sessions.remove(&client_session_key);
        }
        return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_start_failed");
    }
    let mut data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(_) => {
            if let Ok(mut sessions) = wechat_ui_login_sessions().lock() {
                sessions.remove(&client_session_key);
            }
            return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_start_invalid");
        }
    };
    let Some(provider_session_key) = data
        .get("session_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    else {
        if let Ok(mut sessions) = wechat_ui_login_sessions().lock() {
            sessions.remove(&client_session_key);
        }
        return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_start_invalid");
    };
    {
        let mut sessions = match wechat_ui_login_sessions().lock() {
            Ok(sessions) => sessions,
            Err(_) => {
                return wechat_login_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "wechat.login_session_store_unavailable",
                )
            }
        };
        if let Err(error_code) = sessions.attach_provider_session(
            &client_session_key,
            &identity.principal_id,
            provider_session_key,
            current_epoch_seconds(),
        ) {
            return wechat_login_error(StatusCode::CONFLICT, error_code);
        }
    }
    let Some(object) = data.as_object_mut() else {
        return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_start_invalid");
    };
    object.insert(
        "session_key".to_string(),
        Value::String(client_session_key),
    );
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn wechat_login_qr_wait(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WechatQrWaitRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let base = match wechatd_base_url(&state) {
        Ok(base) => base,
        Err(resp) => return resp,
    };
    let login_session = {
        let mut sessions = match wechat_ui_login_sessions().lock() {
            Ok(sessions) => sessions,
            Err(_) => {
                return wechat_login_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "wechat.login_session_store_unavailable",
                )
            }
        };
        match sessions.resolve(
            req.session_key.trim(),
            &identity.principal_id,
            current_epoch_seconds(),
        ) {
            Ok(session) => session,
            Err(error_code) => return wechat_login_error(StatusCode::FORBIDDEN, error_code),
        }
    };
    let provider_session_key = login_session.provider_session_key.unwrap_or_default();
    let url = format!("{}/login/qr/wait", base.trim_end_matches('/'));
    let resp = match state
        .core
        .http_client
        .post(&url)
        .json(&json!({
            "session_key": provider_session_key,
            "timeout_ms": req.timeout_ms.unwrap_or(1_500)
        }))
        .send()
        .await
    {
        Ok(v) => v,
        Err(_) => return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_wait_unavailable"),
    };
    if !resp.status().is_success() {
        return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_wait_failed");
    }
    let mut data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(_) => return wechat_login_error(StatusCode::BAD_GATEWAY, "wechat.qr_wait_invalid"),
    };
    let connected = data
        .get("connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if connected {
        let Some(external_user_id) = wechat_external_user_id(&data).map(ToString::to_string)
        else {
            return wechat_login_error(
                StatusCode::BAD_GATEWAY,
                "wechat.confirmed_identity_missing",
            );
        };
        match bind_channel_identity(
            &state,
            "wechat",
            Some(&external_user_id),
            Some(&external_user_id),
            &identity.user_key,
        ) {
            Ok(Some(bound_identity))
                if bound_identity.principal_id == identity.principal_id => {}
            Ok(_) => {
                return wechat_login_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "wechat.auto_bind_failed",
                )
            }
            Err(_) => {
                return wechat_login_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "wechat.auto_bind_failed",
                )
            }
        }
        if let Ok(mut sessions) = wechat_ui_login_sessions().lock() {
            sessions.remove(req.session_key.trim());
        }
        if let Some(object) = data.as_object_mut() {
            object.remove("user_id");
            object.insert("provider_connected".to_string(), Value::Bool(true));
            object.insert("current_user_bound".to_string(), Value::Bool(true));
            object.insert(
                "binding_status".to_string(),
                Value::String("bound".to_string()),
            );
        }
    } else if matches!(
        data.get("qr_status").and_then(Value::as_str),
        Some("expired")
    ) {
        if let Ok(mut sessions) = wechat_ui_login_sessions().lock() {
            sessions.remove(req.session_key.trim());
        }
    }
    if let Some(object) = data.as_object_mut() {
        object.remove("user_id");
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

async fn whatsapp_web_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let base = state
        .channels
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.not_configured".to_string()),
            }),
        );
    }
    let url = format!("{base}/v1/logout");
    let resp = match state.core.http_client.post(&url).send().await {
        Ok(v) => v,
        Err(_err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("whatsapp_web.logout_unavailable".to_string()),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.logout_failed".to_string()),
            }),
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({ "ok": true })),
            error: None,
        }),
    )
}

async fn local_interaction_context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<LocalInteractionContext>>) {
    match require_ui_identity(&state, &headers) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(LocalInteractionContext {
                    user_id: identity.user_id,
                    chat_id: identity.chat_id,
                    role: identity.role,
                }),
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
