use super::protocol::*;
use crate::{session::Session, transport::bytes_body, Result};
use futures_util::StreamExt;
use http::{HeaderMap, Method};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

pub(crate) use super::now;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

pub(crate) fn backend_error(status: u16, code: Option<&str>) -> &'static str {
    match code {
        Some("asset_owner_unsupported") => "wallet_backend_unsupported",
        Some("nni_asset_owner_mismatch") => "wallet_backend_account_restricted",
        Some("asset_owner_insufficient_balance") => "wallet_insufficient_balance",
        Some("asset_owner_challenge_expired") => "wallet_operation_expired",
        Some("asset_owner_context_changed" | "asset_owner_context_invalid") => {
            "wallet_node_changed"
        }
        Some("asset_owner_operation_conflict" | "asset_owner_operation_completed") => {
            "wallet_unresolved_operation"
        }
        Some("asset_owner_fee_changed" | "asset_owner_slippage_exceeded") => "wallet_quote_changed",
        Some("asset_owner_same_account") => "wallet_same_account",
        Some("asset_owner_amount_too_small") => "wallet_amount_too_small",
        Some("asset_owner_market_unavailable") => "wallet_market_unavailable",
        Some("asset_owner_account_frozen") => "wallet_account_frozen",
        Some("asset_owner_system_account_reserved") => "wallet_system_account_reserved",
        Some("asset_owner_busy" | "asset_owner_rate_limited") => "wallet_rate_limited",
        Some("asset_owner_unavailable") => "wallet_network_failed",
        _ if status == 429 => "wallet_rate_limited",
        _ if matches!(status, 404 | 405 | 501)
            && code.is_none_or(|c| {
                matches!(c, "not_found" | "webd_route_not_found" | "unsupported")
            }) =>
        {
            "wallet_backend_unsupported"
        }
        _ if matches!(status, 401 | 403) => "wallet_access_denied",
        _ if status >= 500 => "wallet_network_failed",
        _ => "wallet_backend_rejected",
    }
}

pub(crate) fn submission_error(error: &str) -> &'static str {
    match error {
        "wallet_rate_limited" => "wallet_outcome_rate_limited",
        "wallet_node_changed" => "wallet_outcome_node_changed",
        _ => "wallet_outcome_unknown",
    }
}

pub async fn request<T: DeserializeOwned>(
    session: &Session,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<T> {
    let info = session.info().await;
    if info
        .identity
        .as_ref()
        .and_then(|v| v.get("role"))
        .and_then(Value::as_str)
        != Some("admin")
    {
        return Err("wallet_admin_required".into());
    }
    let headers = HeaderMap::from_iter([(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    )]);
    let response = tokio::time::timeout(Duration::from_secs(15), async {
        let mut response = session
            .request(
                method,
                path,
                headers,
                body.map(|b| bytes_body(b.to_string())),
            )
            .await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.body.next().await {
            let chunk = chunk.map_err(|_| "wallet_network_failed")?;
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err::<T, String>("wallet_response_invalid".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !(200..300).contains(&response.status) {
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            return Err(
                backend_error(response.status, value.get("error").and_then(Value::as_str)).into(),
            );
        }
        let envelope: Envelope<T> =
            serde_json::from_slice(&bytes).map_err(|_| "wallet_response_invalid")?;
        if !envelope.ok || envelope.error.is_some() {
            return Err("wallet_backend_rejected".into());
        }
        envelope.data.ok_or("wallet_response_invalid".into())
    })
    .await
    .map_err(|_| "wallet_network_failed")??;
    if session.cancelled.is_cancelled() {
        return Err("stale_connection".into());
    }
    Ok(response)
}

pub async fn capabilities(
    session: &Session,
    service: Service,
    action: &str,
) -> Result<Capabilities> {
    let cap: Capabilities = request(
        session,
        Method::GET,
        &format!("{API}/capabilities?service={}", service.name()),
        None,
    )
    .await?;
    cap.validate(service, action)?;
    Ok(cap)
}
pub async fn challenge(
    session: &Session,
    cap: &Capabilities,
    account: &str,
    id: Uuid,
    intent: &Intent,
) -> Result<(Payload, String)> {
    intent.validate(cap.service, account)?;
    let route = if intent.write() { "operations" } else { "read" };
    let response:Challenge=request(session,Method::POST,&format!("{API}/{route}/request"),Some(json!({
        "schema_version":1,"protocol":PROTOCOL,"ledger_id":cap.ledger_id,"node_url":cap.node_url,"service":cap.service,
        "account":account,"operation_id":id,"intent":intent,
    }))).await?;
    let payload = validate_challenge(&response.signing_payload, cap, account, id, intent, now())?;
    Ok((payload, response.signing_payload))
}
pub fn verify_body(payload: &Payload, signature: &str) -> Value {
    json!({"schema_version":1,"protocol":PROTOCOL,"ledger_id":payload.ledger_id,"node_url":payload.node_url,
        "service":payload.service,"account":payload.account,"operation_id":payload.operation_id,"challenge_id":payload.challenge_id,"signature":signature})
}

pub async fn public_read<T: DeserializeOwned>(
    session: &Session,
    cap: &Capabilities,
    account: &str,
    intent: &Intent,
) -> Result<T> {
    intent.validate(cap.service, account)?;
    if intent.write() {
        return Err("wallet_intent_invalid".into());
    }
    request(session, Method::POST, &format!("{API}/read/public"), Some(json!({
        "schema_version":1,"protocol":PROTOCOL,"ledger_id":cap.ledger_id,"node_url":cap.node_url,
        "service":cap.service,"account":account,"operation_id":Uuid::new_v4(),"intent":intent,
    }))).await
}
