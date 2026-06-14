use super::*;

pub(super) fn markdown_to_plain_text(text: &str) -> String {
    static CODE_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    static IMAGE_RE: OnceLock<Regex> = OnceLock::new();
    static LINK_RE: OnceLock<Regex> = OnceLock::new();
    static TABLE_SEP_RE: OnceLock<Regex> = OnceLock::new();
    static HEADING_RE: OnceLock<Regex> = OnceLock::new();
    static QUOTE_RE: OnceLock<Regex> = OnceLock::new();
    static LIST_RE: OnceLock<Regex> = OnceLock::new();
    static ORDERED_LIST_RE: OnceLock<Regex> = OnceLock::new();
    static BOLD_STAR_RE: OnceLock<Regex> = OnceLock::new();
    static BOLD_UNDERSCORE_RE: OnceLock<Regex> = OnceLock::new();
    static ITALIC_STAR_RE: OnceLock<Regex> = OnceLock::new();
    static ITALIC_UNDERSCORE_RE: OnceLock<Regex> = OnceLock::new();

    let mut result = text.replace("\r\n", "\n");
    result = CODE_BLOCK_RE
        .get_or_init(|| Regex::new(r"(?s)```[^\n]*\n?(.*?)```").expect("valid code block regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = IMAGE_RE
        .get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("valid image regex"))
        .replace_all(&result, "")
        .into_owned();
    result = LINK_RE
        .get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").expect("valid link regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = TABLE_SEP_RE
        .get_or_init(|| Regex::new(r"(?m)^\|[\s:|-]+\|$").expect("valid table separator regex"))
        .replace_all(&result, "")
        .into_owned();

    let mut lines = Vec::new();
    for line in result.lines() {
        let trimmed = line.trim();
        let mut normalized =
            if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
                trimmed[1..trimmed.len() - 1]
                    .split('|')
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join("  ")
            } else {
                line.to_string()
            };
        normalized = HEADING_RE
            .get_or_init(|| Regex::new(r"^\s{0,3}#{1,6}\s+").expect("valid heading regex"))
            .replace(&normalized, "")
            .into_owned();
        normalized = QUOTE_RE
            .get_or_init(|| Regex::new(r"^\s*>\s?").expect("valid quote regex"))
            .replace(&normalized, "")
            .into_owned();
        normalized = LIST_RE
            .get_or_init(|| Regex::new(r"^\s*[-*+]\s+").expect("valid list regex"))
            .replace(&normalized, "")
            .into_owned();
        normalized = ORDERED_LIST_RE
            .get_or_init(|| Regex::new(r"^\s*\d+\.\s+").expect("valid ordered list regex"))
            .replace(&normalized, "")
            .into_owned();
        lines.push(normalized);
    }

    result = lines.join("\n");
    result = BOLD_STAR_RE
        .get_or_init(|| Regex::new(r"\*\*([^*\n]+)\*\*").expect("valid bold star regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = BOLD_UNDERSCORE_RE
        .get_or_init(|| Regex::new(r"__([^_\n]+)__").expect("valid bold underscore regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = ITALIC_STAR_RE
        .get_or_init(|| Regex::new(r"\*([^*\n]+)\*").expect("valid italic star regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = ITALIC_UNDERSCORE_RE
        .get_or_init(|| Regex::new(r"_([^_\n]+)_").expect("valid italic underscore regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = result.replace("~~", "");
    result = result.replace('`', "");

    let mut compact = Vec::new();
    let mut last_blank = false;
    for line in result.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            if !last_blank {
                compact.push(String::new());
            }
            last_blank = true;
        } else {
            compact.push(trimmed.to_string());
            last_blank = false;
        }
    }
    compact.join("\n").trim().to_string()
}

pub(super) async fn send_text_message(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    to_user_id: &str,
    context_token: Option<&str>,
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
        let body = SendMessageReq {
            msg: OutboundMessage {
                from_user_id: String::new(),
                to_user_id: to_user_id.to_string(),
                client_id: format!("wechatd-{}", current_ts_ms()),
                message_type: 2,
                message_state: 2,
                item_list: vec![OutboundMessageItem {
                    r#type: 1,
                    text_item: OutboundTextItem {
                        text: if chunk_count > 1 {
                            format!("（{}/{}）\n{}", index + 1, chunk_count, chunk)
                        } else {
                            chunk
                        },
                    },
                }],
                context_token: Some(context_token.to_string()),
            },
            base_info: ilink::base_info(),
        };
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

pub(super) async fn materialize_wechat_outbound_media(
    state: &State,
    media: &WechatOutboundMedia,
) -> Result<PathBuf, String> {
    match &media.source {
        WechatOutboundSource::LocalPath(path) => Ok(path.clone()),
        WechatOutboundSource::RemoteUrl(url) => {
            download_remote_media_to_temp(
                &state.client,
                url,
                Path::new(WECHAT_MEDIA_OUTBOUND_TEMP_DIR),
                "wechatd",
            )
            .await
        }
    }
}

pub(super) async fn send_wechat_error_notice(
    state: &State,
    base_url: &str,
    token: &str,
    to_user_id: &str,
    context_token: &str,
    err: &str,
) {
    let notice = if err.contains("remote media download failed") || err.contains("fetch") {
        "⚠️ 媒体文件下载失败，请检查链接是否可访问。".to_string()
    } else if err.contains("getuploadurl")
        || err.contains("cdn upload")
        || err.contains("upload_param")
    {
        "⚠️ 媒体文件上传失败，请稍后重试。".to_string()
    } else {
        format!("⚠️ 消息发送失败：{err}")
    };
    if let Err(send_err) = send_text_message(
        &state.client,
        &state.config,
        base_url,
        token,
        to_user_id,
        Some(context_token),
        &notice,
    )
    .await
    {
        warn!(
            "wechatd: send error notice failed to={} original_err={} notice_err={}",
            to_user_id, err, send_err
        );
    }
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
        return Err(body.error.unwrap_or_else(|| "resolve failed".to_string()));
    }
    Ok(body.data.and_then(|d| d.identity))
}

pub(super) async fn bind_wechat_identity(
    client: &Client,
    base_url: &str,
    external_user_id: &str,
    external_chat_id: &str,
    user_key: &str,
) -> Result<Option<AuthIdentity>, String> {
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
    let body: ApiResponse<AuthIdentity> = resp
        .json()
        .await
        .map_err(|e| format!("bind response parse failed: {e}"))?;
    if status.as_u16() == 401 || !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}
