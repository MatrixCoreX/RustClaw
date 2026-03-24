//! HTTP 反向代理：对外监听，转发至本机 `clawd`；可选 `/webd/login` 会话并注入 `X-RustClaw-Key`。

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use claw_core::config::AppConfig;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    upstream: String,
    client: reqwest::Client,
    forward_x_forwarded: bool,
    max_incoming_body_bytes: usize,
    cookie_name: String,
    session_ttl_secs: u64,
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
}

struct SessionEntry {
    user_key: String,
    expires_unix: u64,
}

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

    let upstream = config.webd.upstream.trim().to_string();
    if upstream.is_empty() {
        anyhow::bail!("[webd].upstream is empty");
    }

    let sessions = Arc::new(Mutex::new(HashMap::<String, SessionEntry>::new()));
    let state = AppState {
        upstream,
        client,
        forward_x_forwarded: config.webd.forward_x_forwarded,
        max_incoming_body_bytes: config.webd.max_incoming_body_bytes.max(1),
        cookie_name: config.webd.session_cookie_name.clone(),
        session_ttl_secs: config.webd.session_ttl_seconds.max(60),
        sessions,
    };

    let listen = config.webd.listen.trim().to_string();
    let app = Router::new()
        .route("/webd/login", post(webd_login).options(webd_options))
        .route("/webd/logout", post(webd_logout).options(webd_options))
        .route("/webd/session", get(webd_session).options(webd_options))
        .fallback(proxy_handler)
        .with_state(state);

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

#[derive(Debug, Deserialize)]
struct WebdLoginBody {
    username: String,
    password: String,
}

async fn webd_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WebdLoginBody>,
) -> impl IntoResponse {
    let origin = cors_allow_origin_from_headers(&headers);
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
            return with_cors(
                (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "ok": false, "error": format!("upstream: {}", e) })),
            )
                .into_response(),
                origin.as_ref(),
            );
        }
    };
    let status = res.status();
    let text = match res.text().await {
        Ok(t) => t,
        Err(e) => {
            return with_cors(
                (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "ok": false, "error": format!("read body: {}", e) })),
            )
                .into_response(),
                origin.as_ref(),
            );
        }
    };
    let val: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            return with_cors(
                (
                status,
                Json(json!({ "ok": false, "error": "invalid JSON from clawd", "raw": text })),
            )
                .into_response(),
                origin.as_ref(),
            );
        }
    };
    if !val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("login failed");
        return with_cors(
            (status, Json(json!({ "ok": false, "error": err }))).into_response(),
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
            return with_cors(
                (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": "missing user_key in clawd response" })),
            )
                .into_response(),
                origin.as_ref(),
            );
        }
    };
    let sid = Uuid::new_v4().to_string();
    let expires = now_unix_secs() + state.session_ttl_secs;
    {
        let mut guard = state.sessions.lock().expect("sessions mutex");
        guard.insert(
            sid.clone(),
            SessionEntry {
                user_key,
                expires_unix: expires,
            },
        );
    }
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        state.cookie_name, sid, state.session_ttl_secs
    );
    let mut res = Json(json!({
        "ok": true,
        "data": { "logged_in": true }
    }))
    .into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    with_cors(res, origin.as_ref())
}

async fn webd_logout(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    let origin = cors_allow_origin_from_headers(req.headers());
    if let Some(sid) = extract_session_id(req.headers(), &state.cookie_name) {
        let mut guard = state.sessions.lock().expect("sessions mutex");
        guard.remove(&sid);
    }
    let clear = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        state.cookie_name
    );
    let mut res = Json(json!({
        "ok": true,
        "data": { "logged_in": false }
    }))
    .into_response();
    if let Ok(v) = HeaderValue::from_str(&clear) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    with_cors(res, origin.as_ref())
}

async fn webd_session(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    let origin = cors_allow_origin_from_headers(req.headers());
    let logged_in = session_user_key(&state, req.headers()).is_some();
    with_cors(
        Json(json!({ "ok": true, "data": { "logged_in": logged_in } })).into_response(),
        origin.as_ref(),
    )
}

async fn webd_options(headers: HeaderMap) -> impl IntoResponse {
    let origin = cors_allow_origin_from_headers(&headers);
    let mut res = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()));
    if let Some(req_headers) = headers.get(header::ACCESS_CONTROL_REQUEST_HEADERS).cloned() {
        res.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, req_headers);
    } else {
        res.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type, x-rustclaw-key"),
        );
    }
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,POST,PUT,PATCH,DELETE,OPTIONS"),
    );
    with_cors(res, origin.as_ref())
}

fn extract_session_id(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let cookie = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some((name, value)) = part.split_once('=') {
            if name.trim() == cookie_name {
                let v = value.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn session_user_key(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let sid = extract_session_id(headers, &state.cookie_name)?;
    let mut guard = state.sessions.lock().expect("sessions mutex");
    let now = now_unix_secs();
    guard.retain(|_, v| v.expires_unix > now);
    let entry = guard.get(&sid)?;
    if entry.expires_unix <= now {
        guard.remove(&sid);
        return None;
    }
    Some(entry.user_key.clone())
}

async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> Result<Response, Infallible> {
    Ok(proxy_inner(state, addr, req).await)
}

async fn proxy_inner(state: AppState, client_addr: SocketAddr, req: Request) -> Response {
    if req.method() == axum::http::Method::OPTIONS {
        return webd_options(req.headers().clone()).await.into_response();
    }
    let origin = cors_allow_origin_from_headers(req.headers());
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let base = state.upstream.trim_end_matches('/');
    let full_url = format!("{}{}", base, path_and_query);

    let session_key = session_user_key(&state, req.headers());

    let incoming_headers = req.headers();
    let upstream_host = match upstream_host_header(&state.upstream) {
        Ok(h) => h,
        Err(msg) => {
            error!("invalid [webd].upstream: {}", msg);
            return plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid webd upstream URL",
                origin.as_ref(),
            );
        }
    };

    let out_headers = build_outgoing_headers(
        incoming_headers,
        &upstream_host,
        client_addr,
        state.forward_x_forwarded,
        session_key.as_deref(),
    );

    let body_in = req.into_body();
    let bytes = match to_bytes(body_in, state.max_incoming_body_bytes).await {
        Ok(b) => b,
        Err(e) => {
            error!("request body over limit or read error: {}", e);
            return plain_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body too large for webd proxy",
                origin.as_ref(),
            );
        }
    };

    let rb = state.client.request(method.clone(), &full_url).headers(out_headers);
    let rb = if bytes.is_empty() {
        rb
    } else {
        rb.body(bytes)
    };

    let res = match rb.send().await {
        Ok(r) => r,
        Err(e) => {
            error!("upstream request failed (url={}): {}", full_url, e);
            return plain_error(
                StatusCode::BAD_GATEWAY,
                &format!("upstream request failed: {}", e),
                origin.as_ref(),
            );
        }
    };

    let status = res.status();
    let resp_headers = sanitize_response_headers(res.headers());
    let stream = res.bytes_stream().map(|r| {
        r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    });
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
                "proxy response build failed",
                origin.as_ref(),
            )
        }
    }
}

fn plain_error(status: StatusCode, msg: &str, origin: Option<&HeaderValue>) -> Response {
    with_cors((status, msg.to_string()).into_response(), origin)
}

fn cors_allow_origin_from_headers(headers: &HeaderMap) -> Option<HeaderValue> {
    headers.get(header::ORIGIN).cloned()
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
            .insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}

fn upstream_host_header(upstream: &str) -> Result<String, &'static str> {
    let u = upstream.trim();
    let after_scheme = u
        .find("://")
        .map(|i| &u[i + 3..])
        .unwrap_or(u);
    let host_port = after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .trim();
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
        if session_user_key.is_some() && k.as_str().eq_ignore_ascii_case("x-rustclaw-key") {
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
            if let Ok(name) = HeaderName::from_bytes(b"x-rustclaw-key") {
                out.insert(name, v);
            }
        }
    }

    if forward_x {
        let ip = client_addr.ip().to_string();
        let merged = if let Some(existing) = incoming.get("x-forwarded-for") {
            format!("{}, {}", existing.to_str().unwrap_or(""), ip)
        } else {
            ip
        };
        if let Ok(v) = HeaderValue::from_str(&merged) {
            if let Ok(name) = HeaderName::from_bytes(b"x-forwarded-for") {
                out.insert(name, v);
            }
        }
        if let Ok(name) = HeaderName::from_bytes(b"x-forwarded-proto") {
            out.insert(name, HeaderValue::from_static("http"));
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
