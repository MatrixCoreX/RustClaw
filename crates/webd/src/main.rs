//! HTTP 反向代理：对外监听，转发至本机 `clawd`；可选 `/webd/login` 会话并注入规范认证头。

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, get_service, post};
use axum::Json;
use axum::Router;
use claw_core::config::AppConfig;
use claw_core::product_identity::AUTH_KEY_HEADER;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

mod request_limits;
mod session_store;

use request_limits::{classify_request, LimitRejection, RequestClass, RequestLimits};

#[derive(Clone)]
struct AppState {
    upstream: String,
    client: reqwest::Client,
    long_running_client: reqwest::Client,
    forward_x_forwarded: bool,
    cookie_name: String,
    session_ttl_secs: u64,
    session_store_path: PathBuf,
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    login_failure_limit: u32,
    login_lockout_secs: u64,
    login_attempts: Arc<Mutex<HashMap<LoginAttemptKey, LoginAttemptEntry>>>,
    request_limits: RequestLimits,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SessionEntry {
    user_key: String,
    session_handle: String,
    username: String,
    role: String,
    #[serde(default)]
    client_ip: String,
    #[serde(default)]
    client_platform: String,
    #[serde(default)]
    user_agent: String,
    created_unix: u64,
    last_activity_unix: u64,
    expires_unix: u64,
    csrf_token: String,
}

#[derive(Clone, Debug)]
struct AuthenticatedSession {
    session_handle: String,
    user_key: String,
    username: String,
    role: String,
    csrf_token: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LoginAttemptKey {
    client_ip: IpAddr,
    username: String,
}

#[derive(Clone, Debug)]
struct LoginAttemptEntry {
    consecutive_failures: u32,
    locked_until_unix: u64,
    last_failure_unix: u64,
}

const LOGIN_ATTEMPT_RETENTION_SECS: u64 = 60 * 60;
const MAX_LOGIN_ATTEMPT_KEYS: usize = 10_000;
const MAX_LOGIN_USERNAME_BYTES: usize = 128;
const MAX_LOGIN_PASSWORD_BYTES: usize = 1024;
const MAX_LOGIN_REQUEST_BODY_BYTES: usize = 8 * 1024;
const MAX_SESSION_CLIENT_IP_BYTES: usize = 64;
const MAX_SESSION_PLATFORM_BYTES: usize = 64;
const MAX_SESSION_USER_AGENT_BYTES: usize = 512;
const WEBD_CSRF_HEADER: &str = "x-agent-csrf-token";
const WEBD_CSRF_TOKEN_HEX_BYTES: usize = 32;
const SESSION_ACTIVITY_PERSIST_INTERVAL_SECS: u64 = 60;
const LOCAL_HTTP_SESSION_COOKIE_SUFFIX: &str = "_local_http";

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .compact()
        .init();

    let config = AppConfig::load("configs/config.toml")?;
    if !config.webd.enabled {
        warn!("webd disabled by config [webd].enabled=false");
        return Ok(());
    }

    let connect = Duration::from_secs(config.webd.connect_timeout_seconds.max(1));
    let request_timeout_secs = if config.webd.request_timeout_seconds > 0 {
        config.webd.request_timeout_seconds
    } else {
        config.server.request_timeout_seconds.max(5)
    };
    let request_timeout = Duration::from_secs(request_timeout_secs);

    let client = reqwest::Client::builder()
        .connect_timeout(connect)
        .timeout(request_timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("build reqwest client failed")?;
    let long_running_client = reqwest::Client::builder()
        .connect_timeout(connect)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("build long-running reqwest client failed")?;

    let upstream = normalize_loopback_upstream(&config.webd.upstream)
        .map_err(|error| anyhow::anyhow!("invalid [webd].upstream: {error}"))?;

    let session_store_path = PathBuf::from(config.webd.session_store_path.trim());
    let restored_sessions = match session_store::load_sessions(&session_store_path, now_unix_secs())
    {
        Ok(sessions) => sessions,
        Err(error) => {
            warn!(error = %error, "webd_session_store_load_failed");
            HashMap::new()
        }
    };
    let sessions = Arc::new(Mutex::new(restored_sessions));
    let login_attempts = Arc::new(Mutex::new(
        HashMap::<LoginAttemptKey, LoginAttemptEntry>::new(),
    ));
    let state = AppState {
        upstream,
        client,
        long_running_client,
        forward_x_forwarded: config.webd.forward_x_forwarded,
        cookie_name: config.webd.session_cookie_name.clone(),
        session_ttl_secs: config.webd.session_ttl_seconds.max(60),
        session_store_path,
        sessions,
        login_failure_limit: config.webd.login_failure_limit.max(1),
        login_lockout_secs: config.webd.login_lockout_seconds.max(1),
        login_attempts,
        request_limits: RequestLimits::new(
            config.webd.request_limits.clone(),
            config.webd.max_incoming_body_bytes,
        ),
    };

    let listen = config.webd.listen.trim().to_string();
    let ui_dist_dir = resolve_ui_dist_dir();
    let ui_index_path = ui_dist_dir.join("index.html");
    if ui_index_path.exists() {
        info!("webd UI static assets enabled at {}", ui_dist_dir.display());
    } else {
        warn!("webd UI static assets missing: {}", ui_index_path.display());
    }
    let app = build_webd_router(state, ui_dist_dir);

    let listener = match TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            error!(
                "webd bind failed on {}: {}. Check if the port conflicts with clawd or channel daemons (feishu/lark/wechat/whatsapp, etc.).",
                listen, e
            );
            return Err(anyhow::anyhow!(e));
        }
    };

    info!(
        "webd listening on {} -> upstream {}",
        listen, config.webd.upstream
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("axum serve failed")?;
    Ok(())
}

fn build_webd_router(state: AppState, ui_dist_dir: PathBuf) -> Router {
    let ui_index_path = ui_dist_dir.join("index.html");
    let ui_service =
        get_service(ServeDir::new(ui_dist_dir).not_found_service(ServeFile::new(ui_index_path)))
            .layer(SetResponseHeaderLayer::if_not_present(
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, max-age=0"),
            ));
    Router::new()
        .route("/webd/login", post(webd_login).options(webd_options))
        .route("/webd/logout", post(webd_logout).options(webd_options))
        .route("/webd/session", get(webd_session).options(webd_options))
        .route("/webd/sessions", get(webd_sessions).options(webd_options))
        .route(
            "/webd/sessions/revoke",
            post(webd_revoke_session).options(webd_options),
        )
        .route("/v1", any(proxy_handler))
        .route("/v1/*path", any(proxy_handler))
        .fallback_service(ui_service)
        .with_state(state)
        .layer(middleware::map_response(add_security_headers_to_response))
}

async fn add_security_headers_to_response(mut response: Response) -> Response {
    apply_security_headers(&mut response);
    response
}

fn resolve_ui_dist_dir() -> PathBuf {
    claw_core::product_identity::env_string("UI_DIST")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from("UI/dist"))
}

#[derive(Debug, Deserialize)]
struct WebdLoginBody {
    username: String,
    password: String,
}

async fn webd_login(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> impl IntoResponse {
    let (parts, incoming_body) = req.into_parts();
    let headers = parts.headers;
    let origin = match require_valid_origin(&headers) {
        Ok(Some(origin)) => Some(origin),
        Ok(None) | Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_required", None);
        }
    };
    if let Err(error_code) = state
        .request_limits
        .validate_headers(&headers, RequestClass::Login)
    {
        return request_envelope_error_response(error_code, origin.as_ref());
    }
    let client_ip = login_client_ip(&headers, client_addr, state.forward_x_forwarded);
    let _request_lease = match state.request_limits.try_acquire(
        client_ip,
        None,
        RequestClass::Login,
        now_unix_secs(),
    ) {
        Ok(lease) => lease,
        Err(rejection) => return request_limit_response(rejection, origin.as_ref()),
    };
    if !request_has_json_content_type(&headers) {
        return webd_error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "webd_login_content_type_invalid",
            origin.as_ref(),
        );
    }
    let body_bytes = match tokio::time::timeout(
        state.request_limits.body_read_timeout(RequestClass::Login),
        to_bytes(incoming_body, MAX_LOGIN_REQUEST_BODY_BYTES),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            error!(error = %error, "webd_login_body_read_failed");
            return request_envelope_error_response("webd_request_body_too_large", origin.as_ref());
        }
        Err(_) => {
            return request_envelope_error_response(
                "webd_request_body_read_timeout",
                origin.as_ref(),
            );
        }
    };
    let body: WebdLoginBody = match serde_json::from_slice(&body_bytes) {
        Ok(body) => body,
        Err(_) => {
            return webd_error_response(
                StatusCode::BAD_REQUEST,
                "webd_login_body_invalid",
                origin.as_ref(),
            );
        }
    };
    let secure_cookie = request_uses_https(&headers, client_addr, state.forward_x_forwarded);
    let previous_session_ids = extract_session_ids(&headers, &state.cookie_name);
    if !cookie_transport_allowed(&headers, client_addr, state.forward_x_forwarded) {
        return webd_error_response(
            StatusCode::UPGRADE_REQUIRED,
            "webd_https_required",
            origin.as_ref(),
        );
    }
    if !valid_login_input(&body.username, &body.password) {
        return webd_error_response(
            StatusCode::BAD_REQUEST,
            "webd_login_input_invalid",
            origin.as_ref(),
        );
    }
    let attempt_key = LoginAttemptKey {
        client_ip: login_client_ip(&headers, client_addr, state.forward_x_forwarded),
        username: body.username.trim().to_lowercase(),
    };
    let now = now_unix_secs();
    if let Some(retry_after) = login_retry_after(&state, &attempt_key, now) {
        return login_locked_response(retry_after, origin.as_ref());
    }
    let url = format!(
        "{}/v1/internal/webd/verify-login",
        state.upstream.trim_end_matches('/')
    );
    let res = match state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&json!({
            "username": body.username.trim(),
            "password": body.password,
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("webd login upstream error: {}", e);
            return webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_login_upstream_unavailable",
                origin.as_ref(),
            );
        }
    };
    let status = res.status();
    let text = match res.text().await {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "webd_login_upstream_body_read_failed");
            return webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_login_upstream_body_read_failed",
                origin.as_ref(),
            );
        }
    };
    let val: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(error) => {
            error!(error = %error, "webd_login_upstream_response_invalid");
            return webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_login_upstream_response_invalid",
                origin.as_ref(),
            );
        }
    };
    if !val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        if status == StatusCode::UNAUTHORIZED {
            if let Some(retry_after) = record_login_failure(&state, attempt_key, now_unix_secs()) {
                return login_locked_response(retry_after, origin.as_ref());
            }
            return with_cors(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "ok": false,
                        "error": "invalid_credentials",
                        "error_code": "invalid_credentials"
                    })),
                )
                    .into_response(),
                origin.as_ref(),
            );
        }
        let err = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("login_failed");
        return with_cors(
            (
                status,
                Json(json!({
                    "ok": false,
                    "error": err,
                    "error_code": err,
                })),
            )
                .into_response(),
            origin.as_ref(),
        );
    }
    let user_key = match val
        .get("data")
        .and_then(|d| d.get("user_key"))
        .and_then(|v| v.as_str())
    {
        Some(k) => k.to_string(),
        None => {
            error!("webd_login_upstream_user_key_missing");
            return webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_login_upstream_response_invalid",
                origin.as_ref(),
            );
        }
    };
    let role = match val
        .get("data")
        .and_then(|d| d.get("role"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(role) => role.to_string(),
        None => {
            error!("webd_login_upstream_role_missing");
            return webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_login_upstream_response_invalid",
                origin.as_ref(),
            );
        }
    };
    clear_login_failures(&state, &attempt_key);
    let sid = Uuid::new_v4().to_string();
    let session_digest = session_id_digest(&sid);
    let session_handle = Uuid::new_v4().to_string();
    let csrf_token = new_csrf_token();
    let created = now_unix_secs();
    let expires = created + state.session_ttl_secs;
    {
        let mut guard = state.sessions.lock().expect("sessions mutex");
        for previous_session_id in previous_session_ids {
            guard.remove(&session_id_digest(&previous_session_id));
        }
        guard.insert(
            session_digest,
            SessionEntry {
                user_key,
                session_handle,
                username: body.username.trim().to_string(),
                role,
                client_ip: client_ip.to_string(),
                client_platform: bounded_session_header(
                    &headers,
                    &HeaderName::from_static("sec-ch-ua-platform"),
                    MAX_SESSION_PLATFORM_BYTES,
                ),
                user_agent: bounded_session_header(
                    &headers,
                    &header::USER_AGENT,
                    MAX_SESSION_USER_AGENT_BYTES,
                ),
                created_unix: created,
                last_activity_unix: created,
                expires_unix: expires,
                csrf_token: csrf_token.clone(),
            },
        );
        persist_session_snapshot(&state, &guard);
    }
    let cookie = session_cookie_value(
        &session_cookie_name(&state.cookie_name, secure_cookie),
        &sid,
        state.session_ttl_secs,
        secure_cookie,
    );
    let mut res = Json(json!({
        "ok": true,
        "data": {
            "logged_in": true,
            "csrf_token": csrf_token,
        }
    }))
    .into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    with_cors(with_no_store(res), origin.as_ref())
}

fn login_client_ip(
    headers: &HeaderMap,
    client_addr: SocketAddr,
    trust_forwarded_from_loopback: bool,
) -> IpAddr {
    if !trust_forwarded_from_loopback || !client_addr.ip().is_loopback() {
        return client_addr.ip();
    }
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .and_then(|value| value.parse::<IpAddr>().ok())
        .unwrap_or_else(|| client_addr.ip())
}

fn prune_login_attempts(attempts: &mut HashMap<LoginAttemptKey, LoginAttemptEntry>, now: u64) {
    attempts.retain(|_, entry| {
        entry.locked_until_unix > now
            || now.saturating_sub(entry.last_failure_unix) <= LOGIN_ATTEMPT_RETENTION_SECS
    });
}

fn login_retry_after(state: &AppState, key: &LoginAttemptKey, now: u64) -> Option<u64> {
    let mut attempts = state.login_attempts.lock().expect("login attempts mutex");
    prune_login_attempts(&mut attempts, now);
    let entry = attempts.get(key)?;
    if entry.locked_until_unix > now {
        return Some(entry.locked_until_unix - now);
    }
    if entry.locked_until_unix != 0 {
        attempts.remove(key);
    }
    None
}

fn record_login_failure(state: &AppState, key: LoginAttemptKey, now: u64) -> Option<u64> {
    let mut attempts = state.login_attempts.lock().expect("login attempts mutex");
    prune_login_attempts(&mut attempts, now);
    if attempts.len() >= MAX_LOGIN_ATTEMPT_KEYS && !attempts.contains_key(&key) {
        if let Some(oldest_key) = attempts
            .iter()
            .filter(|(_, entry)| entry.locked_until_unix <= now)
            .min_by_key(|(_, entry)| entry.last_failure_unix)
            .map(|(key, _)| key.clone())
        {
            attempts.remove(&oldest_key);
        } else {
            return Some(state.login_lockout_secs);
        }
    }
    let entry = attempts.entry(key).or_insert(LoginAttemptEntry {
        consecutive_failures: 0,
        locked_until_unix: 0,
        last_failure_unix: now,
    });
    entry.last_failure_unix = now;
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    if entry.consecutive_failures < state.login_failure_limit {
        return None;
    }
    entry.locked_until_unix = now.saturating_add(state.login_lockout_secs);
    Some(state.login_lockout_secs)
}

fn clear_login_failures(state: &AppState, key: &LoginAttemptKey) {
    state
        .login_attempts
        .lock()
        .expect("login attempts mutex")
        .remove(key);
}

fn login_locked_response(retry_after: u64, origin: Option<&HeaderValue>) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "ok": false,
            "error": "login_temporarily_locked",
            "error_code": "login_temporarily_locked",
            "data": { "retry_after_seconds": retry_after }
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    with_cors(response, origin)
}

fn request_limit_response(rejection: LimitRejection, origin: Option<&HeaderValue>) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "ok": false,
            "data": {
                "owner_layer": "webd",
                "error_code": rejection.error_code,
                "status_code": rejection.error_code,
                "retry_after_seconds": rejection.retry_after_seconds,
            },
            "error": rejection.error_code,
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&rejection.retry_after_seconds.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    with_cors(response, origin)
}

fn request_envelope_error_response(
    error_code: &'static str,
    origin: Option<&HeaderValue>,
) -> Response {
    let status = match error_code {
        "webd_request_headers_too_large" => StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
        "webd_request_body_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
        "webd_request_content_encoding_unsupported" => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "webd_request_body_read_timeout" => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::BAD_REQUEST,
    };
    webd_error_response(status, error_code, origin)
}

async fn webd_logout(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> impl IntoResponse {
    let origin = match require_valid_origin(req.headers()) {
        Ok(Some(origin)) => Some(origin),
        Ok(None) | Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_required", None);
        }
    };
    if cookie_auth_requires_https(req.headers())
        && !cookie_transport_allowed(req.headers(), client_addr, state.forward_x_forwarded)
    {
        return webd_error_response(
            StatusCode::UPGRADE_REQUIRED,
            "webd_https_required",
            origin.as_ref(),
        );
    }
    let session = authenticated_session(&state, req.headers());
    if let Some(session) = session.as_ref() {
        if let Err(error_code) = require_session_csrf(req.headers(), &session.csrf_token) {
            return webd_error_response(StatusCode::FORBIDDEN, error_code, origin.as_ref());
        }
    }
    let session_id = session
        .as_ref()
        .map(|session| session.session_handle.as_str());
    let client_ip = login_client_ip(req.headers(), client_addr, state.forward_x_forwarded);
    let _request_lease = match state.request_limits.try_acquire(
        client_ip,
        session_id,
        RequestClass::General,
        now_unix_secs(),
    ) {
        Ok(lease) => lease,
        Err(rejection) => return request_limit_response(rejection, origin.as_ref()),
    };
    let secure_cookie = request_uses_https(req.headers(), client_addr, state.forward_x_forwarded);
    let session_ids = extract_session_ids(req.headers(), &state.cookie_name);
    if !session_ids.is_empty() {
        let mut guard = state.sessions.lock().expect("sessions mutex");
        let mut removed = false;
        for sid in session_ids {
            removed |= guard.remove(&session_id_digest(&sid)).is_some();
        }
        if removed {
            persist_session_snapshot(&state, &guard);
        }
    }
    let mut res = Json(json!({
        "ok": true,
        "data": { "logged_in": false }
    }))
    .into_response();
    for cookie_name in session_cookie_names(&state.cookie_name) {
        let clear = session_cookie_value(&cookie_name, "", 0, secure_cookie);
        if let Ok(value) = HeaderValue::from_str(&clear) {
            res.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    with_cors(with_no_store(res), origin.as_ref())
}

async fn webd_session(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> impl IntoResponse {
    let origin = match require_valid_origin(req.headers()) {
        Ok(origin) => origin,
        Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_not_allowed", None);
        }
    };
    if cookie_auth_requires_https(req.headers())
        && !cookie_transport_allowed(req.headers(), client_addr, state.forward_x_forwarded)
    {
        return webd_error_response(
            StatusCode::UPGRADE_REQUIRED,
            "webd_https_required",
            origin.as_ref(),
        );
    }
    let mut session = authenticated_session(&state, req.headers());
    if let Some(current) = session.as_ref() {
        match inspect_upstream_identity(&state, &current.user_key).await {
            Ok(role) if role.eq_ignore_ascii_case(&current.role) => {}
            Ok(_) => {
                revoke_session_by_handle(&state, &current.session_handle);
                session = None;
            }
            Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
                revoke_session_by_handle(&state, &current.session_handle);
                session = None;
            }
            Err(response) => return with_cors(response, origin.as_ref()),
        }
    }
    let session_id = session
        .as_ref()
        .map(|session| session.session_handle.as_str());
    let client_ip = login_client_ip(req.headers(), client_addr, state.forward_x_forwarded);
    let _request_lease = match state.request_limits.try_acquire(
        client_ip,
        session_id,
        RequestClass::General,
        now_unix_secs(),
    ) {
        Ok(lease) => lease,
        Err(rejection) => return request_limit_response(rejection, origin.as_ref()),
    };
    with_cors(
        with_no_store(
            Json(json!({
                "ok": true,
                "data": {
                    "logged_in": session.is_some(),
                    "csrf_token": session.as_ref().map(|session| session.csrf_token.as_str()),
                    "username": session.as_ref().map(|session| session.username.as_str()),
                    "role": session.as_ref().map(|session| session.role.as_str()),
                }
            }))
            .into_response(),
        ),
        origin.as_ref(),
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeWebdSessionBody {
    session_handle: String,
}

async fn webd_sessions(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> impl IntoResponse {
    let origin = match require_valid_origin(req.headers()) {
        Ok(origin) => origin,
        Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_not_allowed", None);
        }
    };
    let session = match authorize_webd_session_admin(&state, req.headers(), client_addr).await {
        Ok(session) => session,
        Err(response) => return with_cors(response, origin.as_ref()),
    };
    let now = now_unix_secs();
    let mut guard = state.sessions.lock().expect("sessions mutex");
    let before = guard.len();
    guard.retain(|_, entry| entry.expires_unix > now);
    if guard.len() != before {
        persist_session_snapshot(&state, &guard);
    }
    let mut sessions = guard
        .values()
        .map(|entry| {
            json!({
                "session_handle": entry.session_handle,
                "username": entry.username,
                "role": entry.role,
                "client_ip": entry.client_ip,
                "client_platform": entry.client_platform,
                "user_agent": entry.user_agent,
                "created_unix": entry.created_unix,
                "last_activity_unix": entry.last_activity_unix,
                "expires_unix": entry.expires_unix,
                "current": session.as_ref().is_some_and(|current| current.session_handle == entry.session_handle),
            })
        })
        .collect::<Vec<_>>();
    sessions.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .get("last_activity_unix")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )
    });
    with_cors(
        with_no_store(
            Json(json!({ "ok": true, "data": { "sessions": sessions } })).into_response(),
        ),
        origin.as_ref(),
    )
}

async fn webd_revoke_session(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> impl IntoResponse {
    let origin = match require_valid_origin(req.headers()) {
        Ok(Some(origin)) => Some(origin),
        Ok(None) | Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_required", None);
        }
    };
    let current = match authorize_webd_session_admin(&state, req.headers(), client_addr).await {
        Ok(session) => session,
        Err(response) => return with_cors(response, origin.as_ref()),
    };
    if let Some(session) = current.as_ref() {
        if let Err(error_code) = require_session_csrf(req.headers(), &session.csrf_token) {
            return webd_error_response(StatusCode::FORBIDDEN, error_code, origin.as_ref());
        }
    }
    if !request_has_json_content_type(req.headers()) {
        return webd_error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "webd_session_revoke_content_type_invalid",
            origin.as_ref(),
        );
    }
    let body = match to_bytes(req.into_body(), MAX_LOGIN_REQUEST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return webd_error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "webd_session_revoke_body_invalid",
                origin.as_ref(),
            );
        }
    };
    let body: RevokeWebdSessionBody = match serde_json::from_slice::<RevokeWebdSessionBody>(&body) {
        Ok(body) if Uuid::parse_str(body.session_handle.trim()).is_ok() => body,
        _ => {
            return webd_error_response(
                StatusCode::BAD_REQUEST,
                "webd_session_revoke_body_invalid",
                origin.as_ref(),
            );
        }
    };
    let mut guard = state.sessions.lock().expect("sessions mutex");
    let target = guard
        .iter()
        .find(|(_, entry)| entry.session_handle == body.session_handle)
        .map(|(digest, _)| digest.clone());
    let revoked = target
        .as_ref()
        .is_some_and(|digest| guard.remove(digest).is_some());
    if revoked {
        persist_session_snapshot(&state, &guard);
    }
    with_cors(
        with_no_store(Json(json!({ "ok": true, "data": { "revoked": revoked } })).into_response()),
        origin.as_ref(),
    )
}

async fn webd_options(headers: HeaderMap) -> impl IntoResponse {
    let origin = match require_valid_origin(&headers) {
        Ok(Some(origin)) => Some(origin),
        Ok(None) | Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_not_allowed", None);
        }
    };
    let mut res = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()));
    if let Some(req_headers) = headers.get(header::ACCESS_CONTROL_REQUEST_HEADERS).cloned() {
        res.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, req_headers);
    } else {
        let allowed = std::iter::once("content-type")
            .chain(std::iter::once(AUTH_KEY_HEADER))
            .chain(std::iter::once(WEBD_CSRF_HEADER))
            .collect::<Vec<_>>()
            .join(", ");
        if let Ok(value) = HeaderValue::from_str(&allowed) {
            res.headers_mut()
                .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, value);
        }
    }
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,POST,PUT,PATCH,DELETE,OPTIONS"),
    );
    with_cors(res, origin.as_ref())
}

fn session_cookie_name(base_name: &str, secure: bool) -> String {
    if secure {
        base_name.to_string()
    } else {
        format!("{base_name}{LOCAL_HTTP_SESSION_COOKIE_SUFFIX}")
    }
}

fn session_cookie_names(base_name: &str) -> [String; 2] {
    [base_name.to_string(), session_cookie_name(base_name, false)]
}

fn extract_session_ids(headers: &HeaderMap, base_cookie_name: &str) -> Vec<String> {
    let accepted_names = session_cookie_names(base_cookie_name);
    let mut session_ids = Vec::new();
    for cookie in headers.get_all(axum::http::header::COOKIE) {
        let Ok(cookie) = cookie.to_str() else {
            continue;
        };
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some((name, value)) = part.split_once('=') {
                if accepted_names
                    .iter()
                    .any(|accepted| name.trim() == accepted)
                {
                    let value = value.trim();
                    if !value.is_empty() && !session_ids.iter().any(|current| current == value) {
                        session_ids.push(value.to_string());
                    }
                }
            }
        }
    }
    session_ids
}

fn session_id_digest(session_id: &str) -> String {
    let digest = Sha256::digest(session_id.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_session_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn bounded_session_header(headers: &HeaderMap, name: &HeaderName, max_bytes: usize) -> String {
    let Some(raw) = headers.get(name).and_then(|value| value.to_str().ok()) else {
        return String::new();
    };
    let sanitized = raw
        .trim()
        .trim_matches('"')
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if sanitized.len() <= max_bytes {
        return sanitized;
    }
    let mut end = max_bytes.min(sanitized.len());
    while !sanitized.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    sanitized[..end].to_string()
}

fn authenticated_session(state: &AppState, headers: &HeaderMap) -> Option<AuthenticatedSession> {
    let session_ids = extract_session_ids(headers, &state.cookie_name);
    if session_ids.is_empty() {
        return None;
    }
    let mut guard = state.sessions.lock().expect("sessions mutex");
    let now = now_unix_secs();
    let before = guard.len();
    guard.retain(|_, v| v.expires_unix > now);
    if guard.len() != before {
        persist_session_snapshot(state, &guard);
    }
    let session_digest = session_ids
        .iter()
        .map(|session_id| session_id_digest(session_id))
        .find(|digest| guard.contains_key(digest))?;
    let entry = guard.get_mut(&session_digest)?;
    let should_persist =
        now.saturating_sub(entry.last_activity_unix) >= SESSION_ACTIVITY_PERSIST_INTERVAL_SECS;
    if should_persist {
        entry.last_activity_unix = now;
    }
    let session = AuthenticatedSession {
        session_handle: entry.session_handle.clone(),
        user_key: entry.user_key.clone(),
        username: entry.username.clone(),
        role: entry.role.clone(),
        csrf_token: entry.csrf_token.clone(),
    };
    if should_persist {
        persist_session_snapshot(state, &guard);
    }
    Some(session)
}

async fn inspect_upstream_identity(state: &AppState, user_key: &str) -> Result<String, Response> {
    let url = format!("{}/v1/auth/me", state.upstream.trim_end_matches('/'));
    let response = state
        .client
        .get(url)
        .header(AUTH_KEY_HEADER, user_key)
        .send()
        .await
        .map_err(|_| {
            webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_identity_upstream_unavailable",
                None,
            )
        })?;
    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN {
        return Err(webd_error_response(
            StatusCode::UNAUTHORIZED,
            "webd_session_invalid",
            None,
        ));
    }
    if !response.status().is_success() {
        return Err(webd_error_response(
            StatusCode::BAD_GATEWAY,
            "webd_identity_upstream_invalid",
            None,
        ));
    }
    let body: Value = response.json().await.map_err(|_| {
        webd_error_response(
            StatusCode::BAD_GATEWAY,
            "webd_identity_upstream_invalid",
            None,
        )
    })?;
    body.get("data")
        .and_then(|data| data.get("role"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|role| !role.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            webd_error_response(
                StatusCode::BAD_GATEWAY,
                "webd_identity_upstream_invalid",
                None,
            )
        })
}

async fn authorize_webd_session_admin(
    state: &AppState,
    headers: &HeaderMap,
    client_addr: SocketAddr,
) -> Result<Option<AuthenticatedSession>, Response> {
    let session = authenticated_session(state, headers);
    if session.is_some()
        && !cookie_transport_allowed(headers, client_addr, state.forward_x_forwarded)
    {
        return Err(webd_error_response(
            StatusCode::UPGRADE_REQUIRED,
            "webd_https_required",
            None,
        ));
    }
    let direct_key = headers
        .get(AUTH_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let user_key = direct_key
        .or_else(|| session.as_ref().map(|session| session.user_key.as_str()))
        .ok_or_else(|| {
            webd_error_response(StatusCode::UNAUTHORIZED, "webd_session_required", None)
        })?;
    let current_role = inspect_upstream_identity(state, user_key).await?;
    if let Some(session) = session.as_ref() {
        if !session.role.eq_ignore_ascii_case(&current_role) {
            revoke_session_by_handle(state, &session.session_handle);
            return Err(webd_error_response(
                StatusCode::UNAUTHORIZED,
                "webd_session_identity_changed",
                None,
            ));
        }
    }
    if !current_role.eq_ignore_ascii_case("admin") {
        return Err(webd_error_response(
            StatusCode::FORBIDDEN,
            "webd_admin_required",
            None,
        ));
    }
    Ok(session)
}

fn revoke_session_by_handle(state: &AppState, session_handle: &str) -> bool {
    let mut guard = state.sessions.lock().expect("sessions mutex");
    let digest = guard
        .iter()
        .find(|(_, entry)| entry.session_handle == session_handle)
        .map(|(digest, _)| digest.clone());
    let revoked = digest
        .as_ref()
        .is_some_and(|digest| guard.remove(digest).is_some());
    if revoked {
        persist_session_snapshot(state, &guard);
    }
    revoked
}

fn new_csrf_token() -> String {
    Uuid::new_v4().simple().to_string()
}

fn valid_csrf_token(value: &str) -> bool {
    value.len() == WEBD_CSRF_TOKEN_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn constant_time_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (*left ^ *right)
        })
        == 0
}

fn require_session_csrf(headers: &HeaderMap, expected_token: &str) -> Result<(), &'static str> {
    let supplied = headers
        .get(WEBD_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or("webd_csrf_token_required")?;
    if !valid_csrf_token(supplied)
        || !constant_time_bytes_equal(supplied.as_bytes(), expected_token.as_bytes())
    {
        return Err("webd_csrf_token_invalid");
    }
    Ok(())
}

fn persist_session_snapshot(state: &AppState, sessions: &HashMap<String, SessionEntry>) {
    if let Err(error) = session_store::persist_sessions(&state.session_store_path, sessions) {
        warn!(error = %error, "webd_session_store_update_failed");
    }
}

async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> Result<Response, Infallible> {
    Ok(proxy_inner(state, addr, req).await)
}

async fn proxy_inner(state: AppState, client_addr: SocketAddr, req: Request) -> Response {
    if is_internal_upstream_path(req.uri().path()) {
        return webd_error_response(StatusCode::NOT_FOUND, "webd_route_not_found", None);
    }
    if req.method() == axum::http::Method::OPTIONS {
        return webd_options(req.headers().clone()).await.into_response();
    }
    let origin = match require_valid_origin(req.headers()) {
        Ok(origin) => origin,
        Err(()) => {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_not_allowed", None);
        }
    };
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let use_long_running_client = uses_long_running_upstream_wait(&method, path_and_query);
    let request_class = classify_request(&method, req.uri().path());
    let base = state.upstream.trim_end_matches('/');
    let full_url = format!("{}{}", base, path_and_query);

    if cookie_auth_requires_https(req.headers())
        && !cookie_transport_allowed(req.headers(), client_addr, state.forward_x_forwarded)
    {
        return webd_error_response(
            StatusCode::UPGRADE_REQUIRED,
            "webd_https_required",
            origin.as_ref(),
        );
    }

    let session = authenticated_session(&state, req.headers());
    if let Some(session) = session.as_ref().filter(|_| method_is_unsafe(&method)) {
        if origin.is_none() {
            return webd_error_response(StatusCode::FORBIDDEN, "webd_origin_required", None);
        }
        if let Err(error_code) = require_session_csrf(req.headers(), &session.csrf_token) {
            return webd_error_response(StatusCode::FORBIDDEN, error_code, origin.as_ref());
        }
    }
    let session_id = session
        .as_ref()
        .map(|session| session.session_handle.as_str());
    let client_ip = login_client_ip(req.headers(), client_addr, state.forward_x_forwarded);
    let request_lease = match state.request_limits.try_acquire(
        client_ip,
        session_id,
        request_class,
        now_unix_secs(),
    ) {
        Ok(lease) => lease,
        Err(rejection) => return request_limit_response(rejection, origin.as_ref()),
    };
    if let Err(error_code) = state
        .request_limits
        .validate_headers(req.headers(), request_class)
    {
        return request_envelope_error_response(error_code, origin.as_ref());
    }

    let incoming_headers = req.headers();
    let upstream_host = match upstream_host_header(&state.upstream) {
        Ok(h) => h,
        Err(msg) => {
            error!("invalid [webd].upstream: {}", msg);
            return plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "webd_upstream_config_invalid",
                origin.as_ref(),
            );
        }
    };

    let out_headers = build_outgoing_headers(
        incoming_headers,
        &upstream_host,
        client_addr,
        state.forward_x_forwarded,
        session.as_ref().map(|session| session.user_key.as_str()),
    );

    let body_in = req.into_body();
    let body_limit = state.request_limits.body_limit(request_class);
    let body_timeout = state.request_limits.body_read_timeout(request_class);
    let bytes = match tokio::time::timeout(body_timeout, to_bytes(body_in, body_limit)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            error!(error = %error, "webd_request_body_read_failed");
            return plain_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "webd_request_body_too_large",
                origin.as_ref(),
            );
        }
        Err(_) => {
            return plain_error(
                StatusCode::REQUEST_TIMEOUT,
                "webd_request_body_read_timeout",
                origin.as_ref(),
            );
        }
    };

    let client = if use_long_running_client {
        &state.long_running_client
    } else {
        &state.client
    };
    let rb = client
        .request(method.clone(), &full_url)
        .headers(out_headers);
    let rb = if bytes.is_empty() { rb } else { rb.body(bytes) };

    let res = match rb.send().await {
        Ok(r) => r,
        Err(e) => {
            error!("upstream request failed (url={}): {}", full_url, e);
            return plain_error(
                StatusCode::BAD_GATEWAY,
                "webd_upstream_unavailable",
                origin.as_ref(),
            );
        }
    };

    let status = res.status();
    if status == StatusCode::UNAUTHORIZED {
        if let Some(session) = session.as_ref() {
            revoke_session_by_handle(&state, &session.session_handle);
        }
    }
    let resp_headers = sanitize_response_headers(res.headers());
    let stream = res
        .bytes_stream()
        .map(|r| r.map_err(|e| std::io::Error::other(e.to_string())));
    let stream = if request_class == RequestClass::Sse {
        stream
            .take_until(tokio::time::sleep(state.request_limits.sse_max_lifetime()))
            .boxed()
    } else {
        stream.boxed()
    };
    let stream = futures_util::stream::unfold(
        (stream, Some(request_lease)),
        |(mut stream, lease)| async move { stream.next().await.map(|item| (item, (stream, lease))) },
    );
    let body = Body::from_stream(stream);

    let mut builder = Response::builder().status(status);
    for (name, value) in resp_headers.iter() {
        builder = builder.header(name, value);
    }
    match builder.body(body) {
        Ok(resp) => with_cors(resp, origin.as_ref()),
        Err(e) => {
            error!("build response failed: {}", e);
            plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "webd_proxy_response_failed",
                origin.as_ref(),
            )
        }
    }
}

fn is_internal_upstream_path(path: &str) -> bool {
    path == "/v1/internal" || path.starts_with("/v1/internal/")
}

fn valid_login_input(username: &str, password: &str) -> bool {
    let username = username.trim();
    !username.is_empty()
        && username.len() <= MAX_LOGIN_USERNAME_BYTES
        && !password.is_empty()
        && password.len() <= MAX_LOGIN_PASSWORD_BYTES
}

fn request_has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn uses_long_running_upstream_wait(method: &axum::http::Method, path_and_query: &str) -> bool {
    let path = path_and_query.split('?').next().unwrap_or(path_and_query);
    if *method == axum::http::Method::POST && path == "/v1/skills/store/install" {
        return true;
    }
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return false;
    }
    let task_suffix = path.strip_prefix("/v1/tasks/");
    let is_event_stream = task_suffix
        .and_then(|suffix| suffix.strip_suffix("/events"))
        .is_some_and(|task_id| !task_id.is_empty() && !task_id.contains('/'));
    let is_artifact_content = task_suffix.is_some_and(|suffix| {
        let parts = suffix.split('/').collect::<Vec<_>>();
        parts.len() == 5
            && !parts[0].is_empty()
            && parts[1] == "artifacts"
            && !parts[2].is_empty()
            && parts[3] == "content"
            && parts[4].is_empty()
    }) || task_suffix.is_some_and(|suffix| {
        let parts = suffix.split('/').collect::<Vec<_>>();
        parts.len() == 4
            && !parts[0].is_empty()
            && parts[1] == "artifacts"
            && !parts[2].is_empty()
            && parts[3] == "content"
    });
    is_event_stream || is_artifact_content
}

fn plain_error(status: StatusCode, error_code: &str, origin: Option<&HeaderValue>) -> Response {
    webd_error_response(status, error_code, origin)
}

fn cors_allow_origin_from_headers(headers: &HeaderMap) -> Option<HeaderValue> {
    let origin = headers.get(header::ORIGIN)?;
    let origin_text = origin.to_str().ok()?.trim();
    let host_text = headers.get(header::HOST)?.to_str().ok()?.trim();
    let origin_url = reqwest::Url::parse(origin_text).ok()?;
    if !matches!(origin_url.scheme(), "http" | "https") {
        return None;
    }
    let request_url = reqwest::Url::parse(&format!("http://{host_text}")).ok()?;
    if origin_url.host_str()? != request_url.host_str()? {
        return None;
    }
    match request_url.port() {
        Some(port) if origin_url.port_or_known_default() != Some(port) => return None,
        None if origin_url.port().is_some() => return None,
        _ => {}
    }
    Some(origin.clone())
}

fn require_valid_origin(headers: &HeaderMap) -> Result<Option<HeaderValue>, ()> {
    if !headers.contains_key(header::ORIGIN) {
        return Ok(None);
    }
    cors_allow_origin_from_headers(headers).map(Some).ok_or(())
}

fn method_is_unsafe(method: &axum::http::Method) -> bool {
    !matches!(
        *method,
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    )
}

fn with_no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

fn webd_error_response(
    status: StatusCode,
    error_code: &str,
    origin: Option<&HeaderValue>,
) -> Response {
    with_cors(
        (
            status,
            Json(json!({
                "ok": false,
                "data": {
                    "owner_layer": "webd",
                    "error_code": error_code,
                    "status_code": error_code,
                },
                "error": error_code,
            })),
        )
            .into_response(),
        origin,
    )
}

fn session_cookie_value(name: &str, value: &str, max_age: u64, secure: bool) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn request_uses_https(
    headers: &HeaderMap,
    client_addr: SocketAddr,
    trust_forwarded_from_loopback: bool,
) -> bool {
    trust_forwarded_from_loopback
        && client_addr.ip().is_loopback()
        && headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
}

fn cookie_transport_allowed(
    headers: &HeaderMap,
    client_addr: SocketAddr,
    trust_forwarded_from_loopback: bool,
) -> bool {
    request_uses_https(headers, client_addr, trust_forwarded_from_loopback)
        || local_http_cookie_client(login_client_ip(
            headers,
            client_addr,
            trust_forwarded_from_loopback,
        ))
}

fn local_http_cookie_client(client_ip: IpAddr) -> bool {
    match client_ip {
        IpAddr::V4(address) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.to_ipv4_mapped().is_some_and(|mapped| {
                    mapped.is_loopback() || mapped.is_private() || mapped.is_link_local()
                })
        }
    }
}

fn cookie_auth_requires_https(headers: &HeaderMap) -> bool {
    headers.contains_key(header::COOKIE) && !headers.contains_key(AUTH_KEY_HEADER)
}

fn with_cors(mut response: Response, origin: Option<&HeaderValue>) -> Response {
    if let Some(origin) = origin {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Origin"));
    }
    apply_security_headers(&mut response);
    response
}

fn apply_security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), geolocation=(), microphone=(self)"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; frame-ancestors 'none'; object-src 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' https: wss:",
        ),
    );
}

fn normalize_loopback_upstream(raw: &str) -> Result<String, &'static str> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "invalid URL")?;
    if url.scheme() != "http" {
        return Err("scheme must be http");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("credentials, query, and fragment are forbidden");
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err("path must be empty");
    }
    let host = url.host_str().ok_or("host is required")?;
    let address = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .map_err(|_| "host must be a loopback IP literal")?;
    if !address.is_loopback() {
        return Err("host must be loopback");
    }
    if url.port().is_none() {
        return Err("an explicit upstream port is required");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn upstream_host_header(upstream: &str) -> Result<String, &'static str> {
    let u = upstream.trim();
    let after_scheme = u.find("://").map(|i| &u[i + 3..]).unwrap_or(u);
    let host_port = after_scheme.split('/').next().unwrap_or("").trim();
    if host_port.is_empty() {
        return Err("empty host");
    }
    Ok(host_port.to_string())
}

fn hop_by_hop_request(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

fn hop_by_hop_response(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
    )
}

fn build_outgoing_headers(
    incoming: &HeaderMap,
    upstream_host: &str,
    client_addr: SocketAddr,
    forward_x: bool,
    session_user_key: Option<&str>,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap as RM, HeaderName, HeaderValue};

    let mut out = RM::new();
    for (k, v) in incoming.iter() {
        if hop_by_hop_request(k.as_str()) {
            continue;
        }
        if session_user_key.is_some() && k.as_str().eq_ignore_ascii_case(AUTH_KEY_HEADER) {
            continue;
        }
        if k.as_str().eq_ignore_ascii_case(WEBD_CSRF_HEADER) {
            continue;
        }
        if k.as_str().eq_ignore_ascii_case("x-forwarded-for") && forward_x {
            continue;
        }
        if k.as_str().eq_ignore_ascii_case("x-forwarded-proto") && forward_x {
            continue;
        }
        if let Ok(name) = HeaderName::from_bytes(k.as_str().as_bytes()) {
            out.append(name, v.clone());
        }
    }

    if let Ok(h) = HeaderValue::from_str(upstream_host) {
        out.insert(reqwest::header::HOST, h);
    }

    if let Some(key) = session_user_key {
        if let Ok(v) = HeaderValue::from_str(key) {
            if let Ok(name) = HeaderName::from_bytes(AUTH_KEY_HEADER.as_bytes()) {
                out.insert(name, v);
            }
        }
    }

    if forward_x {
        let ip = client_addr.ip().to_string();
        let trusted_proxy = client_addr.ip().is_loopback();
        let merged = if trusted_proxy {
            incoming.get("x-forwarded-for").map_or_else(
                || ip.clone(),
                |existing| format!("{}, {}", existing.to_str().unwrap_or(""), ip),
            )
        } else {
            ip
        };
        if let Ok(v) = HeaderValue::from_str(&merged) {
            if let Ok(name) = HeaderName::from_bytes(b"x-forwarded-for") {
                out.insert(name, v);
            }
        }
        if let Ok(name) = HeaderName::from_bytes(b"x-forwarded-proto") {
            let proto = if trusted_proxy
                && incoming
                    .get("x-forwarded-proto")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.split(',').next())
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
            {
                "https"
            } else {
                "http"
            };
            out.insert(name, HeaderValue::from_static(proto));
        }
    }

    out
}

fn sanitize_response_headers(src: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in src.iter() {
        if hop_by_hop_response(k.as_str()) {
            continue;
        }
        if let Ok(name) = HeaderName::from_bytes(k.as_str().as_bytes()) {
            out.append(name, v.clone());
        }
    }
    out
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
