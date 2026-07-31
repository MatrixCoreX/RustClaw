use super::*;

pub(super) fn parse_voice_reply_mode(raw: &str) -> VoiceReplyMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" => VoiceReplyMode::Text,
        "both" => VoiceReplyMode::Both,
        _ => VoiceReplyMode::Voice,
    }
}

pub(super) fn normalize_voice_reply_mode(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "voice" => Some("voice".to_string()),
        "text" => Some("text".to_string()),
        "both" => Some("both".to_string()),
        _ => None,
    }
}

pub(super) fn should_expect_key_reply(state: &BotState, chat_id: i64) -> bool {
    state
        .pending_key_bind_by_chat
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&chat_id))
}

pub(super) fn set_expect_key_reply(state: &BotState, chat_id: i64, enabled: bool) {
    if let Ok(mut set) = state.pending_key_bind_by_chat.lock() {
        if enabled {
            set.insert(chat_id);
        } else {
            set.remove(&chat_id);
        }
    }
}

pub(super) fn extract_bind_key_candidate(text: &str, expect_key_reply: bool) -> Option<String> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("/key")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            if expect_key_reply && !trimmed.is_empty() && !trimmed.starts_with('/') {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
}

pub(super) async fn send_bind_key_required_prompt(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
) -> anyhow::Result<()> {
    set_expect_key_reply(state, msg.chat.id.0, true);
    bot.send_message(
        msg.chat.id,
        state.i18n.t("telegram.msg.bind_key_required_for_chat"),
    )
    .await
    .context("send key bind required prompt failed")?;
    Ok(())
}

pub(super) fn store_bound_identity(state: &BotState, chat_id: i64, identity: &AuthIdentity) {
    if let Ok(mut map) = state.bound_identity_by_chat.lock() {
        map.insert(chat_id, identity.clone());
    }
}

pub(super) fn bound_user_key_for_chat(state: &BotState, chat_id: i64) -> Option<String> {
    state
        .bound_identity_by_chat
        .lock()
        .ok()
        .and_then(|map| map.get(&chat_id).map(|identity| identity.user_key.clone()))
}

pub(super) fn maybe_with_user_key_header(
    req: reqwest::RequestBuilder,
    user_key: Option<&str>,
) -> reqwest::RequestBuilder {
    if let Some(k) = user_key.map(str::trim).filter(|v| !v.is_empty()) {
        req.header("X-Agent-Key", k)
    } else {
        req
    }
}

pub(super) fn normalize_telegram_username(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('@').trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

pub(super) fn telegram_user_allowed(
    state: &BotState,
    user_id: i64,
    username: Option<&str>,
) -> bool {
    if state.access_mode != "specified" {
        return true;
    }
    // 已改为靠 key 绑定用户；仅保留 allowlist / allowed_usernames，不再使用 admins 列表
    if state.allowlist.contains(&user_id) {
        return true;
    }
    username
        .and_then(normalize_telegram_username)
        .is_some_and(|name| state.allowed_usernames.contains(&name))
}

pub(super) async fn resolve_telegram_identity(
    state: &BotState,
    platform_user_id: i64,
    platform_chat_id: i64,
) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/resolve", state.clawd_base_url);
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Telegram,
        telegram_bot_name: None,
        external_user_id: Some(platform_user_id.to_string()),
        external_chat_id: Some(platform_chat_id.to_string()),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp.json().await?;
    if !status.is_success() || !body.ok {
        return Err(anyhow!(telegram_provider_invalid_response(
            "resolve_identity",
            body.error.as_deref().unwrap_or("application_rejected"),
        )));
    }
    Ok(body.data.and_then(|v| v.identity))
}

pub(super) async fn bind_telegram_identity(
    state: &BotState,
    platform_user_id: i64,
    platform_chat_id: i64,
    user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/bind", state.clawd_base_url);
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Telegram,
        telegram_bot_name: None,
        external_user_id: Some(platform_user_id.to_string()),
        external_chat_id: Some(platform_chat_id.to_string()),
        user_key: user_key.trim().to_string(),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<AuthIdentity> = resp.json().await?;
    if !status.is_success() {
        if status.as_u16() == 401 {
            return Ok(None);
        }
        return Err(anyhow!(telegram_provider_invalid_response(
            "bind_identity",
            body.error.as_deref().unwrap_or("application_rejected"),
        )));
    }
    if !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

pub(super) fn pending_resume_valid_for(
    pending: &PendingResumeContext,
    user_id: i64,
    now_secs: u64,
) -> bool {
    if pending.user_id != user_id {
        return false;
    }
    now_secs.saturating_sub(pending.created_at_secs) <= RESUME_CONTEXT_TTL_SECONDS
}

pub(super) async fn maybe_handle_resume_continuation(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    prompt: &str,
) -> anyhow::Result<bool> {
    let chat_id = msg.chat.id.0;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let pending = {
        let guard = state
            .pending_resume_by_chat
            .lock()
            .map_err(|_| anyhow!("pending resume lock poisoned"))?;
        guard.get(&chat_id).cloned()
    };
    let Some(pending) = pending else {
        return Ok(false);
    };
    if !pending_resume_valid_for(&pending, user_id, now_secs) {
        if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
            guard.remove(&chat_id);
        }
        return Ok(false);
    }
    let payload = json!({
        "text": prompt,
        "source": "resume_continue_execute",
        "resume_user_text": prompt,
        "resume_context": pending.resume_context,
    });
    match submit_task_only(
        state,
        user_id,
        chat_id,
        Some(msg.id.0.to_string()),
        TaskKind::Ask,
        payload,
    )
    .await
    {
        Ok(task_id) => {
            spawn_task_result_delivery(
                bot.clone(),
                state.clone(),
                msg.chat.id,
                user_id,
                task_id,
                None,
                state.i18n.t("telegram.msg.process_failed"),
            );
            Ok(true)
        }
        Err(err) => {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.process_failed",
                    &[("error", &err.to_string())],
                ),
            )
            .await
            .context("send resume submit error failed")?;
            Ok(true)
        }
    }
}

pub(super) fn effective_voice_reply_mode_for_chat(state: &BotState, chat_id: i64) -> String {
    let fallback =
        normalize_voice_reply_mode(&state.voice_reply_mode).unwrap_or_else(|| "voice".to_string());
    if let Ok(map) = state.voice_reply_mode_by_chat.lock() {
        if let Some(mode) = map
            .get(&chat_id)
            .and_then(|v| normalize_voice_reply_mode(v))
        {
            return mode;
        }
    }
    fallback
}
