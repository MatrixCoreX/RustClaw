use super::{read_nni_runtime_config, require_ui_admin, AppState};
use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{OriginalUri, Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use claw_core::owner_gateway_context;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const UPSTREAM: &str = "/v1/nni/server/assets/owner";
const MAX_BODY: usize = 16_384;
const MAX_RESPONSE: usize = 256 * 1024;

#[path = "nni_owner_response.rs"]
mod response;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/nni/assets/owner/capabilities", get(handle))
        .route("/nni/assets/owner/read/request", post(handle))
        .route("/nni/assets/owner/read/public", post(handle))
        .route("/nni/assets/owner/read/verify", post(handle))
        .route("/nni/assets/owner/operations/request", post(handle))
        .route("/nni/assets/owner/operations/verify", post(handle))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestEnvelope {
    schema_version: u32,
    protocol: String,
    ledger_id: String,
    node_url: String,
    service: String,
    account: String,
    operation_id: uuid::Uuid,
    intent: Option<Value>,
    challenge_id: Option<uuid::Uuid>,
    signature: Option<String>,
}
impl RequestEnvelope {
    fn validate(&self, verify: bool) -> bool {
        self.schema_version == 1
            && self.protocol == "asset_owner_v1"
            && !self.ledger_id.is_empty()
            && self.ledger_id.len() <= 128
            && !self.operation_id.is_nil()
            && self.account.len() <= 64
            && super::normalize_nni_owner_public_key(&self.account).is_ok()
            && if verify {
                self.intent.is_none()
                    && self.challenge_id.is_some_and(|id| !id.is_nil())
                    && self.signature.as_ref().is_some_and(|s| {
                        s.len() == 128
                            && s.bytes()
                                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    })
            } else {
                self.intent.as_ref().is_some_and(Value::is_object)
                    && self.challenge_id.is_none()
                    && self.signature.is_none()
            }
    }
}

fn failure(status: StatusCode, code: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"ok":false,"error":code})),
    )
        .into_response()
}

fn canonical_origin(raw: &str) -> Result<(String, bool), ()> {
    let url = reqwest::Url::parse(raw).map_err(|_| ())?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]")) && url.scheme() == "http";
    if !local
        && url
            .host_str()
            .and_then(|s| s.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| !crate::public_http_client::is_public_ip(ip))
    {
        return Err(());
    }
    if !local {
        crate::public_http_client::validate_public_https_base_url(raw).map_err(|_| ())?;
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(());
    }
    Ok((url.origin().ascii_serialization(), local))
}

async fn handle(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    req: Request,
) -> Response {
    let identity = match require_ui_admin(&state, req.headers()) {
        Ok(identity) => identity,
        Err((status, _)) => return failure(status, "asset_owner_access_denied"),
    };
    let path = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    let suffix = match uri.path().strip_prefix("/v1/nni/assets/owner/") {
        Some(suffix) => suffix,
        None => return failure(StatusCode::NOT_FOUND, "asset_owner_unsupported"),
    };
    let assertion = match req.headers().get(owner_gateway_context::HEADER) {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value.to_owned()),
            Err(_) => return failure(StatusCode::FORBIDDEN, "asset_owner_context_invalid"),
        },
        None => None,
    };
    let method = req.method().clone();
    let bytes =
        match tokio::time::timeout(Duration::from_secs(3), to_bytes(req.into_body(), MAX_BODY))
            .await
        {
            Ok(Ok(bytes)) => bytes,
            _ => return failure(StatusCode::BAD_REQUEST, "asset_owner_request_invalid"),
        };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let Some(binding) = owner_gateway_context::binding(
        &identity.user_key,
        assertion.as_deref(),
        method.as_str(),
        path,
        &bytes,
        now,
    ) else {
        return failure(StatusCode::FORBIDDEN, "asset_owner_context_invalid");
    };
    let envelope;
    let service = if suffix == "capabilities" {
        let query_url = match reqwest::Url::parse(&format!(
            "http://localhost/?{}",
            uri.query().unwrap_or("")
        )) {
            Ok(url) => url,
            Err(_) => return failure(StatusCode::BAD_REQUEST, "asset_owner_request_invalid"),
        };
        let pairs: Vec<(String, String)> = query_url.query_pairs().into_owned().collect();
        if pairs.len() != 1 || pairs[0].0 != "service" || !bytes.is_empty() {
            return failure(StatusCode::BAD_REQUEST, "asset_owner_request_invalid");
        }
        envelope = None;
        pairs[0].1.clone()
    } else {
        let body: RequestEnvelope = match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) => return failure(StatusCode::BAD_REQUEST, "asset_owner_request_invalid"),
        };
        if uri.query().is_some() || !body.validate(suffix.ends_with("/verify")) {
            return failure(StatusCode::BAD_REQUEST, "asset_owner_request_invalid");
        }
        let service = body.service.clone();
        envelope = Some(body);
        service
    };
    let config = match read_nni_runtime_config(&state) {
        Ok(config) => config,
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "asset_owner_unavailable"),
    };
    let selected = match service.as_str() {
        "assets" => config.asset_service_node_url.as_ref(),
        "bancor" => config.bancor_service_node_url.as_ref(),
        _ => return failure(StatusCode::BAD_REQUEST, "asset_owner_request_invalid"),
    }
    .or(config.selected_node_url.as_ref())
    .or_else(|| config.remote_nodes.first());
    let Some((origin, local)) = selected.and_then(|node| canonical_origin(node).ok()) else {
        return failure(StatusCode::CONFLICT, "asset_owner_context_changed");
    };
    if envelope
        .as_ref()
        .is_some_and(|body| body.node_url != origin)
    {
        return failure(StatusCode::CONFLICT, "asset_owner_context_changed");
    }
    let client = if local {
        static LOCAL: OnceLock<reqwest::Client> = OnceLock::new();
        LOCAL.get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("local HTTP client")
        })
    } else {
        &state.core.public_http_client
    };
    let mut request = client
        .request(method, format!("{origin}{UPSTREAM}/{suffix}"))
        .timeout(Duration::from_secs(10))
        .header(header::ACCEPT, "application/json");
    if suffix == "capabilities" {
        request = request.query(&[("service", service.as_str()), ("node_url", origin.as_str())]);
    } else {
        request = request
            .header("x-agent-owner-context", binding)
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
    }
    match tokio::time::timeout(
        Duration::from_secs(11),
        forward(request, suffix, &service, &origin, envelope.as_ref()),
    )
    .await
    {
        Ok(Ok(response)) => response,
        _ => failure(StatusCode::SERVICE_UNAVAILABLE, "asset_owner_unavailable"),
    }
}

async fn forward(
    request: reqwest::RequestBuilder,
    suffix: &str,
    service: &str,
    origin: &str,
    envelope: Option<&RequestEnvelope>,
) -> Result<Response, ()> {
    let mut upstream = request.send().await.map_err(|_| ())?;
    let status = upstream.status();
    if status.is_redirection() {
        return Err(());
    }
    let retry = upstream
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(|s| s.clamp(1, 300));
    let mut bytes = Vec::new();
    while let Some(chunk) = upstream.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let code = value.get("error").and_then(Value::as_str).filter(|s| {
            s.starts_with("asset_owner_")
                && s.len() < 80
                && s.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
        });
        let mut response = failure(
            status,
            code.unwrap_or(if matches!(status.as_u16(), 404 | 405 | 501) {
                "asset_owner_unsupported"
            } else if status.as_u16() == 429 {
                "asset_owner_rate_limited"
            } else {
                "asset_owner_unavailable"
            }),
        );
        if let Some(seconds) = retry {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, seconds.into());
        }
        return Ok(response);
    }
    let bytes = response::project(&bytes, suffix, service, origin, envelope)?;
    Ok((
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from(Bytes::from(bytes)),
    )
        .into_response())
}

#[cfg(test)]
#[path = "nni_owner_financial_tests.rs"]
mod tests;
