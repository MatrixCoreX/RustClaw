use super::*;

pub(super) fn session_token(
    config: &WechatSection,
    session: Option<&PersistedSession>,
) -> Option<String> {
    if let Some(existing) = session {
        if !existing.bot_token.trim().is_empty() {
            return Some(existing.bot_token.trim().to_string());
        }
    }
    let config_token = config.bot_token.trim();
    if config_token.is_empty() || config_token == "REPLACE_ME" {
        None
    } else {
        Some(config_token.to_string())
    }
}

pub(super) fn session_base_url(
    config: &WechatSection,
    session: Option<&PersistedSession>,
) -> String {
    session
        .and_then(|s| s.base_url.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| config.api_base_url.trim())
        .to_string()
}

pub(super) async fn fetch_qrcode(
    client: &Client,
    section: &WechatSection,
    bot_type: &str,
) -> Result<QRCodeResponse, String> {
    let base = format!("{}/", section.api_base_url.trim_end_matches('/'));
    let url = format!("{}ilink/bot/get_bot_qrcode?bot_type={}", base, bot_type);
    let req = client.get(&url);
    let response = ilink::apply_route_tag(req, section)
        .send()
        .await
        .map_err(|e| format!("fetch QR code failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("fetch QR code status={status} body={body}"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("parse QR code response failed: {e}"))
}

pub(super) async fn poll_qr_status(
    client: &Client,
    section: &WechatSection,
    qrcode: &str,
) -> Result<QRStatusResponse, String> {
    let base = format!("{}/", section.api_base_url.trim_end_matches('/'));
    let url = format!("{}ilink/bot/get_qrcode_status?qrcode={}", base, qrcode);
    let req = client.get(&url).header("iLink-App-ClientVersion", "1");
    let response = ilink::apply_route_tag(req, section)
        .send()
        .await
        .map_err(|e| format!("poll QR status failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("poll QR status={status} body={body}"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("parse QR status failed: {e}"))
}

pub(super) async fn get_updates(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    get_updates_buf: &str,
    timeout_ms: u64,
) -> Result<GetUpdatesResp, String> {
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        "ilink/bot/getupdates"
    );
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", ilink::build_wechat_uin_header(config))
        .json(&GetUpdatesReq {
            get_updates_buf,
            base_info: ilink::base_info(),
        })
        .timeout(Duration::from_millis(timeout_ms.max(1_000)));
    req = ilink::apply_route_tag(req, config);
    let response = match req.send().await {
        Ok(response) => response,
        Err(err) if err.is_timeout() => {
            return Ok(GetUpdatesResp {
                ret: Some(0),
                errcode: None,
                errmsg: None,
                msgs: Vec::new(),
                get_updates_buf: (!get_updates_buf.is_empty())
                    .then_some(get_updates_buf.to_string()),
                longpolling_timeout_ms: None,
            });
        }
        Err(err) => return Err(format!("wechat request failed: {err}")),
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("wechat request status={status} body={body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("getupdates decode failed: {e}"))
}

pub(super) fn normalized_context_token(context_token: Option<&str>) -> Option<&str> {
    context_token.map(str::trim).filter(|v| !v.is_empty())
}

pub(super) fn context_token_store_key(account_id: &str, user_id: &str) -> String {
    format!("{account_id}:{user_id}")
}

pub(super) fn session_account_id(session: Option<&PersistedSession>) -> String {
    session
        .and_then(|s| s.account_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("primary")
        .to_string()
}

pub(super) async fn remember_context_token(state: &State, user_id: &str, token: &str) {
    let account_id = {
        let session = state.session.read().await;
        session_account_id(session.as_ref())
    };
    state.context_tokens.write().await.insert(
        context_token_store_key(&account_id, user_id),
        token.to_string(),
    );
}

pub(super) async fn resolve_delivery_context_token(
    state: &State,
    user_id: &str,
    explicit: Option<&str>,
) -> Option<String> {
    if let Some(token) = normalized_context_token(explicit) {
        remember_context_token(state, user_id, token).await;
        return Some(token.to_string());
    }
    let account_id = {
        let session = state.session.read().await;
        session_account_id(session.as_ref())
    };
    state
        .context_tokens
        .read()
        .await
        .get(&context_token_store_key(&account_id, user_id))
        .cloned()
}
