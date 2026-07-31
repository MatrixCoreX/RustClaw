use super::*;

const WECHAT_BIND_REQUIRED_FOR_CHAT_KEY: &str = "wechat.msg.bind_key_required_for_chat";
const WECHAT_BIND_HELP_KEY: &str = "wechat.msg.bind_help";
const WECHAT_BIND_SUCCESS_KEY: &str = "wechat.msg.bind_success";
const WECHAT_BIND_INVALID_KEY: &str = "wechat.msg.bind_invalid";
const WECHAT_BIND_REQUEST_FAILED_KEY: &str = "wechat.msg.bind_request_failed";

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

pub(super) async fn should_expect_key_reply(
    state: &State,
    scope: &WechatConversationScope,
) -> bool {
    state
        .pending_key_bind_by_user
        .read()
        .await
        .contains(&scope.storage_key())
}

pub(super) async fn set_expect_key_reply(
    state: &State,
    scope: &WechatConversationScope,
    enabled: bool,
) {
    let key = scope.storage_key();
    let mut guard = state.pending_key_bind_by_user.write().await;
    if enabled {
        guard.insert(key);
    } else {
        guard.remove(&key);
    }
}

pub(super) async fn ensure_bound_before_task(
    state: &State,
    scope: &WechatConversationScope,
    from_user_id: &str,
    context_token: Option<&str>,
    text_for_binding: Option<&str>,
) -> Option<AuthIdentity> {
    let identity = match resolve_wechat_identity(
        &state.client,
        &state.config.clawd_base_url,
        from_user_id,
        &scope.storage_key(),
    )
    .await
    {
        Ok(identity) => identity,
        Err(err) => {
            warn!("wechatd: resolve identity failed err={}", err);
            return None;
        }
    };
    let identity = if identity.is_some() {
        identity
    } else {
        // Read-only compatibility for bindings created before account-scoped
        // conversation IDs. New writes always use the scoped key.
        resolve_wechat_identity(
            &state.client,
            &state.config.clawd_base_url,
            from_user_id,
            from_user_id,
        )
        .await
        .ok()
        .flatten()
    };
    if let Some(identity) = identity {
        set_expect_key_reply(state, scope, false).await;
        return Some(identity);
    }

    if let Some(text) = text_for_binding {
        let trimmed = text.trim();
        if is_unbound_allowed_command(trimmed) {
            set_expect_key_reply(state, scope, true).await;
            let reply = wechat_t(&state.config, WECHAT_BIND_HELP_KEY);
            send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
            return None;
        }
        let expect_key_reply = should_expect_key_reply(state, scope).await;
        if let Some(candidate) = extract_bind_key_candidate(trimmed, expect_key_reply) {
            match bind_wechat_identity(
                &state.client,
                &state.config.clawd_base_url,
                from_user_id,
                &scope.storage_key(),
                &candidate,
            )
            .await
            {
                Ok(Some(_)) => {
                    set_expect_key_reply(state, scope, false).await;
                    let reply = wechat_t(&state.config, WECHAT_BIND_SUCCESS_KEY);
                    send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
                }
                Ok(None) => {
                    set_expect_key_reply(state, scope, true).await;
                    let reply = wechat_t(&state.config, WECHAT_BIND_INVALID_KEY);
                    send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
                }
                Err(err) => {
                    warn!("wechatd: bind request failed err={}", err);
                    set_expect_key_reply(state, scope, true).await;
                    let reply = wechat_t(&state.config, WECHAT_BIND_REQUEST_FAILED_KEY);
                    send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
                }
            }
            return None;
        }
    }

    set_expect_key_reply(state, scope, true).await;
    let reply = wechat_t(&state.config, WECHAT_BIND_REQUIRED_FOR_CHAT_KEY);
    send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
    None
}
