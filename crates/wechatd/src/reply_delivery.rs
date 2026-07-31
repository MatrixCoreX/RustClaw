use super::*;

pub(super) async fn send_text_message(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    run_id: Option<&str>,
    text: &str,
) -> Result<(), String> {
    let Some(context_token) = normalized_context_token(context_token) else {
        return Err("sendmessage requires context_token".to_string());
    };
    let chunks = chunk_text_for_channel(
        text,
        config
            .text_chunk_chars
            .max(1)
            .min(WECHAT_TEXT_CHUNK_CHARS)
            .saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let chunk_count = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let text = if chunk_count > 1 {
            format!("（{}/{}）\n{}", index + 1, chunk_count, chunk)
        } else {
            chunk
        };
        let body = WechatSendMessageRequest::finish(
            to_user_id,
            context_token,
            wechat_ilink::new_wechat_client_id("text"),
            run_id.map(str::to_string),
            WechatMessageItem::text(text)?,
            WECHATD_CHANNEL_VERSION,
        )?;
        let _ = ilink::post_json(
            client,
            config,
            base_url,
            token,
            "ilink/bot/sendmessage",
            &body,
            config.request_timeout_seconds.max(1) * 1_000,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn send_text_reply_via_session(
    state: &State,
    to_user_id: &str,
    context_token: Option<&str>,
    text: &str,
) {
    let session_guard = state.session.read().await;
    let token = session_token(&state.config, session_guard.as_ref());
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let Some(token) = token else {
        return;
    };
    let Some(context_token) =
        resolve_delivery_context_token(state, to_user_id, context_token).await
    else {
        return;
    };
    let _ = send_text_message(
        &state.client,
        &state.config,
        &base_url,
        &token,
        to_user_id,
        Some(context_token.as_str()),
        None,
        text,
    )
    .await;
}

pub(super) async fn resolve_wechat_identity(
    client: &Client,
    base_url: &str,
    external_user_id: &str,
    external_chat_id: &str,
) -> Result<Option<AuthIdentity>, String> {
    let url = format!("{}/v1/auth/channel/resolve", base_url.trim_end_matches('/'));
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Wechat,
        external_user_id: Some(external_user_id.to_string()),
        external_chat_id: Some(external_chat_id.to_string()),
        telegram_bot_name: None,
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("resolve request failed: {e}"))?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp
        .json()
        .await
        .map_err(|e| format!("resolve response parse failed: {e}"))?;
    if !status.is_success() || !body.ok {
        return Err(
            claw_core::channel_provider_error::ChannelProviderError::invalid_response(
                "wechat_ilink",
                "resolve_identity",
                body.error.as_deref().unwrap_or("application_rejected"),
            )
            .to_string(),
        );
    }
    Ok(body.data.and_then(|d| d.identity))
}

pub(super) async fn bind_wechat_identity(
    client: &Client,
    base_url: &str,
    external_user_id: &str,
    external_chat_id: &str,
    user_key: &str,
) -> Result<Option<BindChannelKeyResponse>, String> {
    let url = format!("{}/v1/auth/channel/bind", base_url.trim_end_matches('/'));
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Wechat,
        external_user_id: Some(external_user_id.to_string()),
        external_chat_id: Some(external_chat_id.to_string()),
        telegram_bot_name: None,
        user_key: user_key.trim().to_string(),
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("bind request failed: {e}"))?;
    let status = resp.status();
    let body: ApiResponse<BindChannelKeyResponse> = resp
        .json()
        .await
        .map_err(|e| format!("bind response parse failed: {e}"))?;
    if status.as_u16() == 401 || !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}
