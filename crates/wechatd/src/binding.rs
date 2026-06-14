use super::*;

const WECHAT_BIND_REQUIRED_FOR_CHAT: &str = "请先发送你的 key 进行绑定，然后再继续聊天或使用功能。\nPlease send your key first to bind this account before chatting or using features.";
const WECHAT_BIND_HELP: &str = "欢迎使用 RustClaw。\n请先发送 /key <your_key> 完成绑定。\nWelcome to RustClaw.\nPlease send /key <your_key> first to bind this account.";
const WECHAT_BIND_SUCCESS: &str =
    "绑定成功，请重新发送刚才的消息。\nKey bound successfully. Please send your previous message again.";
const WECHAT_BIND_INVALID: &str = "key 无效，请重新输入。\nInvalid key. Please try again.";

pub(super) fn is_unbound_allowed_command(text: &str) -> bool {
    static COMMAND_CATALOG: OnceLock<ChannelCommandCatalog> = OnceLock::new();
    COMMAND_CATALOG
        .get_or_init(ChannelCommandCatalog::default)
        .allows_unbound_command(text, "wechat")
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

pub(super) async fn should_expect_key_reply(state: &State, external_user_id: &str) -> bool {
    state
        .pending_key_bind_by_user
        .read()
        .await
        .contains(external_user_id)
}

pub(super) async fn set_expect_key_reply(state: &State, external_user_id: &str, enabled: bool) {
    let mut guard = state.pending_key_bind_by_user.write().await;
    if enabled {
        guard.insert(external_user_id.to_string());
    } else {
        guard.remove(external_user_id);
    }
}

pub(super) async fn ensure_bound_before_task(
    state: &State,
    from_user_id: &str,
    context_token: Option<&str>,
    text_for_binding: Option<&str>,
) -> Option<AuthIdentity> {
    let identity = match resolve_wechat_identity(
        &state.client,
        &state.config.clawd_base_url,
        from_user_id,
        from_user_id,
    )
    .await
    {
        Ok(identity) => identity,
        Err(err) => {
            warn!("wechatd: resolve identity failed err={}", err);
            return None;
        }
    };
    if let Some(identity) = identity {
        set_expect_key_reply(state, from_user_id, false).await;
        return Some(identity);
    }

    if let Some(text) = text_for_binding {
        let trimmed = text.trim();
        if is_unbound_allowed_command(trimmed) {
            set_expect_key_reply(state, from_user_id, true).await;
            send_text_reply_via_session(state, from_user_id, context_token, WECHAT_BIND_HELP).await;
            return None;
        }
        let expect_key_reply = should_expect_key_reply(state, from_user_id).await;
        if let Some(candidate) = extract_bind_key_candidate(trimmed, expect_key_reply) {
            match bind_wechat_identity(
                &state.client,
                &state.config.clawd_base_url,
                from_user_id,
                from_user_id,
                &candidate,
            )
            .await
            {
                Ok(Some(_)) => {
                    set_expect_key_reply(state, from_user_id, false).await;
                    send_text_reply_via_session(
                        state,
                        from_user_id,
                        context_token,
                        WECHAT_BIND_SUCCESS,
                    )
                    .await;
                }
                Ok(None) => {
                    set_expect_key_reply(state, from_user_id, true).await;
                    send_text_reply_via_session(
                        state,
                        from_user_id,
                        context_token,
                        WECHAT_BIND_INVALID,
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: bind request failed err={}", err);
                    set_expect_key_reply(state, from_user_id, true).await;
                    send_text_reply_via_session(
                        state,
                        from_user_id,
                        context_token,
                        "绑定请求失败，请稍后重试。\nBind request failed, please try again later.",
                    )
                    .await;
                }
            }
            return None;
        }
    }

    set_expect_key_reply(state, from_user_id, true).await;
    send_text_reply_via_session(
        state,
        from_user_id,
        context_token,
        WECHAT_BIND_REQUIRED_FOR_CHAT,
    )
    .await;
    None
}
