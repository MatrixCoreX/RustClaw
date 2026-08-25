mod config;
mod device_proof;
mod openai;
mod quota;
mod store;

use std::{io, sync::Arc};

use anyhow::{bail, Context};
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{NaiveDate, Utc};
use config::{RelayConfig, StoreConfig};
use device_proof::verify_enrollment_signature;
use futures_util::StreamExt;
use openai::{ChatCompletionRequest, ErrorBody, ErrorEnvelope, ModelList};
use quota::MinuteRateLimiter;
use serde::Deserialize;
use serde_json::{json, Value};
use store::{normalize_device_pubkey, AuthenticatedKey, RelayStore, StoreError};
use tokio::sync::Semaphore;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

const MAX_ADMIN_DAILY_REQUEST_LIMIT: u32 = 1_000_000;

#[derive(Clone)]
struct AppState {
    config: Arc<RelayConfig>,
    http: reqwest::Client,
    store: Arc<RelayStore>,
    minute_rate: Arc<MinuteRateLimiter>,
    inflight: Arc<Semaphore>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "llm_relay_server=info,tower_http=info".into()),
        )
        .init();

    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|argument| argument == "key") {
        return run_key_command(&arguments[1..]);
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "device")
    {
        return run_device_command(&arguments[1..]);
    }
    if arguments
        .first()
        .is_some_and(|argument| argument != "serve")
    {
        bail!("usage: llm-relay-server [serve | device allow|list|revoke | key issue-admin|list|revoke]");
    }
    run_server().await
}

async fn run_server() -> anyhow::Result<()> {
    let config = Arc::new(RelayConfig::from_env()?);
    let store = Arc::new(RelayStore::open_and_recover(
        &config.store.database_path,
        &config.store.key_pepper,
    )?);
    let state = AppState {
        config: Arc::clone(&config),
        http: reqwest::Client::builder()
            .timeout(config.upstream_timeout)
            .build()
            .context("failed to build HTTP client")?,
        store,
        minute_rate: Arc::new(MinuteRateLimiter::new(config.limits.requests_per_minute)),
        inflight: Arc::new(Semaphore::new(config.max_inflight)),
    };

    let app = Router::new()
        .route("/health", get(health_live))
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/v1/device-key/request", post(request_device_key))
        .route("/v1/device-key/verify", post(verify_device_key))
        .route("/v1/models", get(models))
        .route("/v1/quota", get(quota))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/internal/admin/usage", get(admin_usage))
        .route(
            "/internal/admin/devices/:device_pubkey/daily-limit",
            put(update_admin_daily_limit),
        )
        .with_state(state)
        .layer(DefaultBodyLimit::max(config.max_request_body_bytes))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.listen_addr))?;
    info!(listen_addr = %config.listen_addr, public_model = %config.default_model, "relay ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn run_key_command(arguments: &[String]) -> anyhow::Result<()> {
    let config = StoreConfig::from_env()?;
    let store = RelayStore::open(&config.database_path, &config.key_pepper)?;
    match arguments.first().map(String::as_str) {
        Some("issue-admin") => {
            let label = option_value(arguments, "--label")
                .ok_or_else(|| anyhow::anyhow!("key issue-admin requires --label"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&store.issue_admin_key(label)?)?
            );
        }
        Some("list") => {
            println!("{}", serde_json::to_string_pretty(&store.list_keys()?)?);
        }
        Some("revoke") => {
            let key_id = arguments
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("key revoke requires a key ID"))?;
            println!(
                "{}",
                json!({"key_id": key_id, "revoked": store.revoke_key(key_id)?})
            );
        }
        _ => bail!(
            "usage: llm-relay-server key issue-admin --label LABEL | key list | key revoke KEY_ID"
        ),
    }
    Ok(())
}

fn run_device_command(arguments: &[String]) -> anyhow::Result<()> {
    let config = StoreConfig::from_env()?;
    let store = RelayStore::open(&config.database_path, &config.key_pepper)?;
    match arguments.first().map(String::as_str) {
        Some("allow") => {
            let label = option_value(arguments, "--label")
                .ok_or_else(|| anyhow::anyhow!("device allow requires --label"))?;
            let device_pubkey = option_value(arguments, "--device-pubkey")
                .ok_or_else(|| anyhow::anyhow!("device allow requires --device-pubkey"))?;
            let daily_limit = option_value(arguments, "--daily-limit")
                .map(str::parse)
                .transpose()
                .context("--daily-limit must be a positive integer")?
                .unwrap_or(config::env_u32("RELAY_REQUESTS_PER_DAY", 100)?);
            println!(
                "{}",
                serde_json::to_string_pretty(&store.allow_device(
                    label,
                    device_pubkey,
                    daily_limit,
                )?)?
            );
        }
        Some("list") => println!(
            "{}",
            serde_json::to_string_pretty(&store.list_allowed_devices()?)?
        ),
        Some("revoke") => {
            let device_pubkey = arguments
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("device revoke requires a Slot 0 public key"))?;
            println!(
                "{}",
                json!({
                    "device_pubkey": device_pubkey,
                    "revoked": store.revoke_device(device_pubkey)?
                })
            );
        }
        _ => bail!("usage: llm-relay-server device allow --label LABEL --device-pubkey SLOT0_PUBKEY [--daily-limit 100] | device list | device revoke SLOT0_PUBKEY"),
    }
    Ok(())
}

fn option_value<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

async fn health_live() -> Json<Value> {
    Json(json!({"ok": true, "service": "llm-relay-server"}))
}

async fn health_ready(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let active_keys = state.store.active_key_count().map_err(|error| {
        warn!(error = %error, "relay readiness check failed");
        ApiError::service_unavailable("relay_store_unavailable", "proxy.relay_store_unavailable")
    })?;
    Ok(Json(json!({
        "ok": true,
        "service": "llm-relay-server",
        "active_key_count": active_keys
    })))
}

#[derive(Debug, Deserialize)]
struct DeviceKeyRequest {
    device_pubkey: String,
}

#[derive(Debug, Deserialize)]
struct DeviceKeyVerifyRequest {
    device_pubkey: String,
    challenge_id: String,
    signature: String,
}

async fn request_device_key(
    State(state): State<AppState>,
    Json(request): Json<DeviceKeyRequest>,
) -> Result<Json<Value>, ApiError> {
    let device_pubkey = normalize_device_pubkey(&request.device_pubkey).map_err(|_| {
        ApiError::bad_request(
            "device_enrollment_invalid",
            "proxy.device_enrollment_invalid",
        )
    })?;
    state.minute_rate.reserve(&format!(
        "enroll:{}",
        device_pubkey
    ))?;
    let challenge = state
        .store
        .create_enrollment_challenge(&device_pubkey)
        .map_err(ApiError::from_store)?;
    Ok(Json(json!({"ok": true, "data": challenge})))
}

async fn verify_device_key(
    State(state): State<AppState>,
    Json(request): Json<DeviceKeyVerifyRequest>,
) -> Result<Json<Value>, ApiError> {
    let device_pubkey = normalize_device_pubkey(&request.device_pubkey).map_err(|_| {
        ApiError::bad_request(
            "device_enrollment_invalid",
            "proxy.device_enrollment_invalid",
        )
    })?;
    state.minute_rate.reserve(&format!(
        "verify:{}",
        device_pubkey
    ))?;
    let challenge = state
        .store
        .pending_enrollment_challenge(&request.challenge_id, &device_pubkey)
        .map_err(ApiError::from_store)?;
    if !verify_enrollment_signature(
        &challenge.device_pubkey,
        &challenge.challenge,
        &request.signature,
    ) {
        return Err(ApiError::unauthorized(
            "device_signature_invalid",
            "proxy.device_signature_invalid",
        ));
    }
    let issued = state
        .store
        .complete_enrollment(&challenge.challenge_id, &challenge.device_pubkey)
        .map_err(ApiError::from_store)?;
    Ok(Json(json!({"ok": true, "data": issued})))
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ModelList>, ApiError> {
    let key = authenticate(&state, &headers)?;
    key.require_scope("models.read")
        .map_err(ApiError::from_store)?;
    Ok(Json(ModelList::from_provider(&state.config.provider)))
}

async fn quota(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let key = authenticate(&state, &headers)?;
    key.require_scope("quota.read")
        .map_err(ApiError::from_store)?;
    let usage = state.store.quota_snapshot(&key).map_err(|error| {
        warn!(error = %error, key_id = %key.key_id, "quota snapshot failed");
        ApiError::service_unavailable("quota_store_unavailable", "proxy.quota_store_unavailable")
    })?;
    Ok(Json(json!({"usage": usage})))
}

#[derive(Debug, Deserialize)]
struct AdminUsageQuery {
    day: Option<String>,
    page: Option<String>,
    per_page: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DailyLimitRequest {
    daily_request_limit: u32,
}

async fn admin_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminUsageQuery>,
) -> Result<Json<Value>, ApiError> {
    let key = authenticate(&state, &headers)?;
    key.require_scope("usage.admin.read")
        .map_err(ApiError::from_store)?;
    let day = match query
        .day
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
            ApiError::bad_request("admin_usage_day_invalid", "proxy.admin_usage_day_invalid")
        })?,
        None => Utc::now().date_naive(),
    };
    let page = parse_admin_page_value(
        query.page.as_deref(),
        1,
        1_000_000,
        "admin_usage_page_invalid",
    )?;
    let per_page = parse_admin_page_value(
        query.per_page.as_deref(),
        50,
        100,
        "admin_usage_per_page_invalid",
    )?;
    let status = query
        .status
        .as_deref()
        .unwrap_or("all")
        .trim()
        .to_ascii_lowercase();
    if !matches!(status.as_str(), "all" | "enabled" | "revoked") {
        return Err(ApiError::bad_request(
            "admin_usage_status_invalid",
            "proxy.admin_usage_status_invalid",
        ));
    }
    let usage = state
        .store
        .admin_usage_page(day, page, per_page, &status)
        .map_err(|error| ApiError::from_store(StoreError::Database(error)))?;
    info!(
        actor_key_id = %key.key_id,
        day = %usage.day_utc,
        page,
        per_page,
        status,
        "relay admin usage read"
    );
    Ok(Json(json!({"ok": true, "data": usage})))
}

async fn update_admin_daily_limit(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(device_pubkey): AxumPath<String>,
    Json(request): Json<DailyLimitRequest>,
) -> Result<Json<Value>, ApiError> {
    let key = authenticate(&state, &headers)?;
    key.require_scope("usage.admin.write")
        .map_err(ApiError::from_store)?;
    if request.daily_request_limit == 0
        || request.daily_request_limit > MAX_ADMIN_DAILY_REQUEST_LIMIT
    {
        return Err(ApiError::bad_request(
            "daily_request_limit_invalid",
            "proxy.daily_request_limit_invalid",
        ));
    }
    let update = state
        .store
        .update_daily_request_limit(
            &key.key_id,
            device_pubkey.trim(),
            request.daily_request_limit,
        )
        .map_err(|error| ApiError::from_store(StoreError::Database(error)))?
        .ok_or_else(|| {
            ApiError::not_found(
                "relay_client_key_not_found",
                "proxy.relay_client_key_not_found",
            )
        })?;
    info!(
        actor_key_id = %key.key_id,
        device_pubkey = %update.device_pubkey,
        previous_daily_request_limit = update.previous_daily_request_limit,
        daily_request_limit = update.daily_request_limit,
        "relay admin daily request limit updated"
    );
    Ok(Json(json!({"ok": true, "data": update})))
}

fn parse_admin_page_value(
    value: Option<&str>,
    default: u32,
    maximum: u32,
    code: &'static str,
) -> Result<u32, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<u32>()
        .ok()
        .filter(|parsed| *parsed > 0 && *parsed <= maximum)
        .ok_or_else(|| ApiError::bad_request(code, "proxy.admin_usage_pagination_invalid"))?;
    Ok(parsed)
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let key = authenticate(&state, &headers)?;
    key.require_scope("chat.completions")
        .map_err(ApiError::from_store)?;
    let request: ChatCompletionRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("request_json_invalid", "proxy.request_json_invalid"))?;
    request.validate(&state.config)?;
    let provider = state
        .config
        .select_provider(request.model())
        .ok_or_else(|| ApiError::bad_request("model_not_allowed", "proxy.model_not_allowed"))?;
    let requested_tokens = request
        .max_tokens()
        .unwrap_or(state.config.limits.max_tokens_per_request.min(4096));
    let upstream_body = request.to_upstream_body(provider);
    let estimated_input_tokens = serde_json::to_vec(&upstream_body)
        .map(|body| body.len().saturating_add(3) / 4)
        .ok()
        .and_then(|tokens| u32::try_from(tokens).ok())
        .unwrap_or(u32::MAX);
    let reserved_tokens = requested_tokens.saturating_add(estimated_input_tokens);
    state.minute_rate.reserve(&key.key_id)?;
    let _permit = state.inflight.clone().acquire_owned().await.map_err(|_| {
        ApiError::service_unavailable("relay_shutting_down", "proxy.relay_shutting_down")
    })?;

    let request_id = Uuid::new_v4().to_string();
    state
        .store
        .reserve_attempt(
            &key,
            &request_id,
            reserved_tokens,
            state.config.limits.tokens_per_day,
            state.config.max_inflight_per_key,
        )
        .map_err(ApiError::from_store)?;

    let upstream_response = state
        .http
        .post(provider.chat_completions_url())
        .bearer_auth(&provider.api_key)
        .header("x-request-id", &request_id)
        .json(&upstream_body)
        .send()
        .await;
    let upstream_response = match upstream_response {
        Ok(response) => response,
        Err(error) => {
            settle_quietly(&state.store, &request_id, false, 0);
            warn!(error = %error, request_id, "upstream request failed");
            return Err(ApiError::bad_gateway(
                "upstream_request_failed",
                "proxy.upstream_request_failed",
            ));
        }
    };
    let status = upstream_response.status();
    if !status.is_success() {
        settle_quietly(&state.store, &request_id, false, 0);
        warn!(
            status = status.as_u16(),
            request_id, "upstream rejected request"
        );
        return Err(ApiError::from_upstream_status(status));
    }

    if request.is_streaming() {
        return Ok(streaming_response(
            upstream_response,
            Arc::clone(&state.store),
            request_id,
            provider.alias.clone(),
            u64::from(reserved_tokens),
        ));
    }

    let mut body: Value = upstream_response.json().await.map_err(|error| {
        settle_quietly(&state.store, &request_id, false, 0);
        warn!(error = %error, request_id, "upstream response was not valid JSON");
        ApiError::bad_gateway("upstream_invalid_json", "proxy.upstream_invalid_json")
    })?;
    let usage = openai::extract_usage(&body);
    let charged_tokens = if usage.total_tokens == 0 {
        u64::from(reserved_tokens)
    } else {
        usage.total_tokens
    };
    settle_quietly(&state.store, &request_id, true, charged_tokens);
    openai::mask_model_name(&mut body, &provider.alias);
    let mut response = Json(body).into_response();
    insert_request_id(response.headers_mut(), &request_id);
    Ok(response)
}

fn streaming_response(
    response: reqwest::Response,
    store: Arc<RelayStore>,
    request_id: String,
    public_model: String,
    fallback_tokens: u64,
) -> Response {
    let mut upstream = response.bytes_stream();
    let guard = StreamSettlement::new(store, request_id.clone(), fallback_tokens);
    let stream = async_stream::stream! {
        let mut guard = guard;
        let mut pending = Vec::new();
        let mut total_tokens = 0_u64;
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk) => {
                    pending.extend_from_slice(&chunk);
                    while let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
                        let line = pending.drain(..=position).collect::<Vec<_>>();
                        let (line, usage) = rewrite_sse_line(line, &public_model);
                        total_tokens = total_tokens.max(usage);
                        guard.observe(usage);
                        yield Ok::<Bytes, io::Error>(Bytes::from(line));
                    }
                }
                Err(error) => {
                    yield Err(io::Error::other(error));
                    return;
                }
            }
        }
        if !pending.is_empty() {
            let (line, usage) = rewrite_sse_line(pending, &public_model);
            total_tokens = total_tokens.max(usage);
            guard.observe(usage);
            yield Ok::<Bytes, io::Error>(Bytes::from(line));
        }
        guard.complete(total_tokens);
    };
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    insert_request_id(response.headers_mut(), &request_id);
    response
}

fn rewrite_sse_line(line: Vec<u8>, public_model: &str) -> (Vec<u8>, u64) {
    let Ok(text) = std::str::from_utf8(&line) else {
        return (line, 0);
    };
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let suffix = &text[trimmed.len()..];
    let Some(payload) = trimmed.strip_prefix("data:").map(str::trim) else {
        return (line, 0);
    };
    if payload == "[DONE]" {
        return (line, 0);
    }
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return (line, 0);
    };
    let usage = openai::extract_usage(&value).total_tokens;
    openai::mask_model_name(&mut value, public_model);
    (format!("data: {}{suffix}", value).into_bytes(), usage)
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthenticatedKey, ApiError> {
    let token = bearer_token(headers)
        .or_else(|| header_value(headers, "x-relay-key"))
        .ok_or_else(|| ApiError::unauthorized("missing_api_key", "proxy.missing_api_key"))?;
    state
        .store
        .authenticate(&token)
        .map_err(ApiError::from_store)
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = header_value(headers, "authorization")?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn insert_request_id(headers: &mut HeaderMap, request_id: &str) {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert("x-request-id", value);
    }
}

fn settle_quietly(store: &RelayStore, request_id: &str, succeeded: bool, total_tokens: u64) {
    if let Err(error) = store.settle_attempt(request_id, succeeded, total_tokens) {
        warn!(error = %error, request_id, "failed to settle relay attempt");
    }
}

struct StreamSettlement {
    store: Arc<RelayStore>,
    request_id: String,
    completed: bool,
    observed_tokens: u64,
    fallback_tokens: u64,
}

impl StreamSettlement {
    fn new(store: Arc<RelayStore>, request_id: String, fallback_tokens: u64) -> Self {
        Self {
            store,
            request_id,
            completed: false,
            observed_tokens: 0,
            fallback_tokens,
        }
    }

    fn observe(&mut self, total_tokens: u64) {
        self.observed_tokens = self.observed_tokens.max(total_tokens);
    }

    fn complete(&mut self, total_tokens: u64) {
        self.observe(total_tokens);
        let charged_tokens = if self.observed_tokens == 0 {
            self.fallback_tokens
        } else {
            self.observed_tokens
        };
        settle_quietly(&self.store, &self.request_id, true, charged_tokens);
        self.completed = true;
    }
}

impl Drop for StreamSettlement {
    fn drop(&mut self) {
        if !self.completed {
            settle_quietly(
                &self.store,
                &self.request_id,
                false,
                self.observed_tokens.max(self.fallback_tokens),
            );
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message_key: &'static str,
}

impl ApiError {
    fn unauthorized(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code,
            message_key,
        }
    }
    fn forbidden(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
            message_key,
        }
    }
    fn not_found(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message_key,
        }
    }
    pub(crate) fn bad_request(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message_key,
        }
    }
    pub(crate) fn too_many_requests(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code,
            message_key,
        }
    }
    fn service_unavailable(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message_key,
        }
    }
    fn bad_gateway(code: &'static str, message_key: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            message_key,
        }
    }
    fn from_store(error: StoreError) -> Self {
        match error {
            StoreError::InvalidKey => {
                Self::unauthorized("invalid_api_key", "proxy.invalid_api_key")
            }
            StoreError::KeyDisabled => {
                Self::unauthorized("api_key_disabled", "proxy.api_key_disabled")
            }
            StoreError::ScopeDenied => Self::forbidden("scope_denied", "proxy.scope_denied"),
            StoreError::DailyRequestLimit => Self::too_many_requests(
                "requests_per_day_exceeded",
                "proxy.requests_per_day_exceeded",
            ),
            StoreError::DailyTokenLimit => {
                Self::too_many_requests("tokens_per_day_exceeded", "proxy.tokens_per_day_exceeded")
            }
            StoreError::KeyInflightLimit => Self::too_many_requests(
                "key_inflight_limit_exceeded",
                "proxy.key_inflight_limit_exceeded",
            ),
            StoreError::DeviceNotAllowed => {
                Self::forbidden("device_not_allowlisted", "proxy.device_not_allowlisted")
            }
            StoreError::EnrollmentInvalid => Self::bad_request(
                "device_enrollment_invalid",
                "proxy.device_enrollment_invalid",
            ),
            StoreError::EnrollmentExpired => Self::unauthorized(
                "device_enrollment_expired",
                "proxy.device_enrollment_expired",
            ),
            StoreError::EnrollmentReplay => Self::unauthorized(
                "device_enrollment_replayed",
                "proxy.device_enrollment_replayed",
            ),
            StoreError::Database(error) => {
                warn!(error = %error, "relay database operation failed");
                Self::service_unavailable(
                    "quota_store_unavailable",
                    "proxy.quota_store_unavailable",
                )
            }
        }
    }
    fn from_upstream_status(status: reqwest::StatusCode) -> Self {
        let translated = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        if translated == StatusCode::TOO_MANY_REQUESTS {
            return Self {
                status: translated,
                code: "upstream_rate_limited",
                message_key: "proxy.upstream_rate_limited",
            };
        }
        if translated == StatusCode::UNAUTHORIZED || translated == StatusCode::FORBIDDEN {
            return Self {
                status: StatusCode::BAD_GATEWAY,
                code: "upstream_auth_failed",
                message_key: "proxy.upstream_auth_failed",
            };
        }
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_error",
            message_key: "proxy.upstream_error",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message_key: self.message_key,
                error_type: "relay",
            },
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests;
