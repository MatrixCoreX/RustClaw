mod config;
mod openai;
mod quota;

use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use config::RelayConfig;
use openai::{ChatCompletionRequest, ErrorBody, ErrorEnvelope, ModelList};
use quota::QuotaManager;
use serde_json::{json, Value};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    config: Arc<RelayConfig>,
    http: reqwest::Client,
    quota: Arc<QuotaManager>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "llm_relay_server=info,tower_http=info".into()),
        )
        .init();

    let config = Arc::new(RelayConfig::from_env()?);
    let configured_providers = config
        .providers
        .iter()
        .filter(|provider| !provider.api_key.is_empty())
        .count();
    if configured_providers == 0 {
        warn!("no upstream API key is configured; proxy calls will return upstream_not_configured");
    }
    if config.api_keys.is_empty() {
        warn!("RELAY_API_KEYS is empty; all authenticated endpoints will reject requests");
    }

    let state = AppState {
        config: Arc::clone(&config),
        http: reqwest::Client::builder()
            .timeout(config.upstream_timeout)
            .build()
            .context("failed to build HTTP client")?,
        quota: Arc::new(QuotaManager::new(config.limits.clone())),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/quota", get(quota))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.listen_addr))?;
    info!("llm relay server listening on {}", config.listen_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "llm-relay-server"
    }))
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ModelList>, ApiError> {
    authenticate(&state.config, &headers)?;
    Ok(Json(ModelList::from_providers(&state.config.providers)))
}

async fn quota(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let client_id = authenticate(&state.config, &headers)?;
    let snapshot = state.quota.snapshot(&client_id);
    Ok(Json(json!({
        "client_id": client_id,
        "limits": state.config.limits,
        "usage": snapshot
    })))
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Json<Value>, ApiError> {
    let client_id = authenticate(&state.config, &headers)?;
    request.validate(&state.config)?;
    let provider = state
        .config
        .select_provider(request.model.as_deref())
        .ok_or_else(|| ApiError::bad_request("model_not_allowed", "proxy.model_not_allowed"))?;

    let requested_max_tokens = request
        .max_tokens
        .unwrap_or(state.config.limits.max_tokens_per_request);
    state.quota.precheck(&client_id, requested_max_tokens)?;

    if provider.api_key.is_empty() {
        return Err(ApiError::service_unavailable(
            "upstream_not_configured",
            "proxy.upstream_not_configured",
        ));
    }

    let upstream_body = request.to_upstream_body(provider);
    let upstream_url = provider.chat_completions_url();
    let upstream_response = state
        .http
        .post(upstream_url)
        .bearer_auth(&provider.api_key)
        .json(&upstream_body)
        .send()
        .await
        .map_err(|err| {
            warn!(error = %err, "upstream request failed");
            ApiError::bad_gateway("upstream_request_failed", "proxy.upstream_request_failed")
        })?;

    let status = upstream_response.status();
    let mut body: Value = upstream_response.json().await.map_err(|err| {
        warn!(error = %err, "upstream response was not valid JSON");
        ApiError::bad_gateway("upstream_invalid_json", "proxy.upstream_invalid_json")
    })?;

    if !status.is_success() {
        state.quota.record_failed_request(&client_id);
        return Err(ApiError::from_upstream_status(status));
    }

    let usage = openai::extract_usage(&body);
    state.quota.settle(&client_id, usage.total_tokens);
    openai::mask_model_name(&mut body, &provider.alias);

    Ok(Json(body))
}

fn authenticate(config: &RelayConfig, headers: &HeaderMap) -> Result<String, ApiError> {
    let token = bearer_token(headers)
        .or_else(|| header_value(headers, "x-relay-key"))
        .ok_or_else(|| ApiError::unauthorized("missing_api_key", "proxy.missing_api_key"))?;

    if config.api_keys.iter().any(|key| key == &token) {
        Ok(token)
    } else {
        Err(ApiError::unauthorized(
            "invalid_api_key",
            "proxy.invalid_api_key",
        ))
    }
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

    fn from_upstream_status(status: reqwest::StatusCode) -> Self {
        let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let code = if status == StatusCode::TOO_MANY_REQUESTS {
            "upstream_rate_limited"
        } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            "upstream_auth_failed"
        } else {
            "upstream_error"
        };
        Self {
            status,
            code,
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
