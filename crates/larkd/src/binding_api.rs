use super::*;

pub(super) async fn resolve_lark_identity(
    client: &Client,
    base_url: &str,
    open_id: &str,
    chat_id: &str,
) -> Result<Option<AuthIdentity>, String> {
    let url = format!("{}/v1/auth/channel/resolve", base_url.trim_end_matches('/'));
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Lark,
        external_user_id: Some(open_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        telegram_bot_name: None,
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("resolve request failed: {}", e))?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp
        .json()
        .await
        .map_err(|e| format!("resolve response parse failed: {}", e))?;
    if !status.is_success() || !body.ok {
        return Err(lark_provider_invalid_response(
            "resolve_identity",
            body.error.as_deref().unwrap_or("application_rejected"),
        ));
    }
    Ok(body.data.and_then(|d| d.identity))
}

pub(super) async fn bind_lark_identity(
    client: &Client,
    base_url: &str,
    open_id: &str,
    chat_id: &str,
    user_key: &str,
) -> Result<Option<BindChannelKeyResponse>, String> {
    let url = format!("{}/v1/auth/channel/bind", base_url.trim_end_matches('/'));
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Lark,
        external_user_id: Some(open_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        telegram_bot_name: None,
        user_key: user_key.trim().to_string(),
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("bind request failed: {}", e))?;
    let status = resp.status();
    let body: ApiResponse<BindChannelKeyResponse> = resp
        .json()
        .await
        .map_err(|e| format!("bind response parse failed: {}", e))?;
    if status.as_u16() == 401 || !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

pub(super) async fn store_pending_lark_request(
    state: &AppState,
    open_id: &str,
    chat_id: &str,
    message_id: &str,
    text: &str,
    attachment: Option<(&str, String)>,
) -> Result<Option<PendingChannelRequestStatus>, String> {
    let prompt = text.trim();
    if (prompt.is_empty() && attachment.is_none()) || message_id.trim().is_empty() {
        return Ok(None);
    }
    let idempotency_key = format!("pending:lark:{}", message_id.trim());
    let mut ingress = claw_core::channel_ingress::ChannelIngressEnvelope::new(
        ChannelKind::Lark,
        open_platform_contract(OpenPlatformRegion::Lark).source_adapter,
    )
    .with_external_ids(open_id.to_string(), chat_id.to_string())
    .with_message_id(message_id.to_string())
    .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::chat(
        chat_id.to_string(),
    ))
    .with_locale(state.config.lark.language.clone());
    if let Some((kind, path)) = attachment {
        ingress
            .attachments
            .push(claw_core::channel_ingress::ChannelIngressAttachment {
                kind: kind.to_string(),
                path,
                mime_type: None,
                size: None,
            });
    }
    let request = SubmitTaskRequest {
        user_id: None,
        chat_id: None,
        user_key: None,
        channel: Some(ChannelKind::Lark),
        external_user_id: Some(open_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        ingress: Some(ingress),
        idempotency_key: Some(idempotency_key.clone()),
        kind: TaskKind::Ask,
        payload: json!({ "text": prompt }),
    };
    let response = state
        .client
        .post(format!(
            "{}/v1/auth/channel/pending-request",
            state.config.lark.clawd_base_url.trim_end_matches('/')
        ))
        .json(&PendingChannelRequestStoreRequest {
            idempotency_key,
            expires_in_seconds: None,
            request,
        })
        .send()
        .await
        .map_err(|error| format!("pending request failed: {error}"))?;
    let status = response.status();
    let body: ApiResponse<PendingChannelRequestStatus> = response
        .json()
        .await
        .map_err(|error| format!("pending response parse failed: {error}"))?;
    if !status.is_success() || !body.ok {
        return Err(lark_provider_invalid_response(
            "store_pending_request",
            body.error.as_deref().unwrap_or("application_rejected"),
        ));
    }
    Ok(body.data)
}

pub(super) fn extract_pending_bind_token_candidate(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if let Some(candidate) = trimmed.strip_prefix("/start").map(str::trim) {
        if candidate.starts_with("pb-") {
            return Some(candidate.to_string());
        }
    }
    trimmed.starts_with("pb-").then(|| trimmed.to_string())
}

pub(super) async fn detect_pending_lark_bind(
    client: &Client,
    base_url: &str,
    open_id: &str,
    chat_id: &str,
    bind_token: &str,
) -> Result<Option<DetectFeishuBindSessionResponse>, String> {
    let url = format!(
        "{}/v1/auth/channel-binds/lark/detect",
        base_url.trim_end_matches('/')
    );
    let req = DetectFeishuBindSessionRequest {
        bind_token: Some(bind_token.trim().to_string()),
        external_user_id: open_id.to_string(),
        external_chat_id: chat_id.to_string(),
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|err| format!("detect request failed: {err}"))?;
    let status = resp.status();
    let body: ApiResponse<DetectFeishuBindSessionResponse> = resp
        .json()
        .await
        .map_err(|err| format!("detect response parse failed: {err}"))?;
    if !status.is_success() || !body.ok {
        return Err(lark_provider_invalid_response(
            "detect_binding",
            body.error.as_deref().unwrap_or("application_rejected"),
        ));
    }
    Ok(body.data.filter(|data| data.matched))
}
