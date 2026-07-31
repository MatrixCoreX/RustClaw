use super::*;

const WECHAT_BIND_REQUIRED_FOR_CHAT_KEY: &str = "wechat.msg.bind_key_required_for_chat";
const WECHAT_BIND_HELP_KEY: &str = "wechat.msg.bind_help";
const WECHAT_BIND_SUCCESS_KEY: &str = "wechat.msg.bind_success";
const WECHAT_PENDING_RESUME_STOPPED_KEY: &str = "wechat.msg.pending_resume_stopped";
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
    task_context: &PinnedWechatTaskContext,
    from_user_id: &str,
    text_for_binding: Option<&str>,
    message_id: Option<&str>,
    attachment_kind: Option<&str>,
) -> Option<AuthIdentity> {
    let scope = &task_context.scope;
    let context_token = Some(task_context.context_token.as_str());
    let explicit_bind_candidate =
        text_for_binding.and_then(|text| extract_bind_key_candidate(text, false));
    let identity = match if explicit_bind_candidate.is_some() {
        Ok(None)
    } else {
        resolve_wechat_identity(
            &state.client,
            &state.config.clawd_base_url,
            from_user_id,
            &scope.storage_key(),
        )
        .await
    } {
        Ok(identity) => identity,
        Err(err) => {
            warn!("wechatd: resolve identity failed err={}", err);
            return None;
        }
    };
    let identity = if explicit_bind_candidate.is_some() || identity.is_some() {
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
        if let Some(candidate) = explicit_bind_candidate
            .or_else(|| extract_bind_key_candidate(trimmed, expect_key_reply))
        {
            match bind_wechat_identity(
                &state.client,
                &state.config.clawd_base_url,
                from_user_id,
                &scope.storage_key(),
                &candidate,
            )
            .await
            {
                Ok(Some(bind_result)) => {
                    set_expect_key_reply(state, scope, false).await;
                    let reply = wechat_t(&state.config, WECHAT_BIND_SUCCESS_KEY);
                    send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
                    if let Some(resume) = bind_result.pending_resume {
                        if let Some(task_id) = resume.task_id {
                            let mut resume_context = task_context.clone();
                            if let Some(token) = resume.context_token {
                                resume_context.context_token = token;
                            }
                            spawn_existing_wechat_task_delivery(
                                state.clone(),
                                resume_context,
                                bind_result.identity.user_key,
                                task_id.to_string(),
                            )
                            .await;
                        } else if resume.error_code.is_some() {
                            let stopped =
                                wechat_t(&state.config, WECHAT_PENDING_RESUME_STOPPED_KEY);
                            send_text_reply_via_session(
                                state,
                                from_user_id,
                                context_token,
                                &stopped,
                            )
                            .await;
                        }
                    }
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
    if let Some(message_id) = message_id {
        if let Err(error) = store_pending_wechat_request(
            state,
            task_context,
            from_user_id,
            message_id,
            text_for_binding.unwrap_or_default(),
            attachment_kind,
        )
        .await
        {
            warn!("wechatd: pending request persistence failed err={error}");
        }
    }

    set_expect_key_reply(state, scope, true).await;
    let reply = wechat_t(&state.config, WECHAT_BIND_REQUIRED_FOR_CHAT_KEY);
    send_text_reply_via_session(state, from_user_id, context_token, &reply).await;
    None
}

async fn store_pending_wechat_request(
    state: &State,
    task_context: &PinnedWechatTaskContext,
    from_user_id: &str,
    message_id: &str,
    text: &str,
    attachment_kind: Option<&str>,
) -> Result<Option<PendingChannelRequestStatus>, String> {
    let prompt = text.trim();
    if (prompt.is_empty() && attachment_kind.is_none()) || message_id.trim().is_empty() {
        return Ok(None);
    }
    let scoped_chat_id = task_context.scope.storage_key();
    let idempotency_key = format!("pending:wechat:{}", message_id.trim());
    let mut ingress = claw_core::channel_ingress::ChannelIngressEnvelope::new(
        ChannelKind::Wechat,
        "wechat_ilink",
    )
    .with_external_ids(from_user_id.to_string(), scoped_chat_id.clone())
    .with_message_id(message_id.to_string())
    .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::user(
        from_user_id.to_string(),
    ))
    .with_locale(state.config.language.clone())
    .with_context_token(task_context.context_token.clone());
    if let Some(kind) = attachment_kind {
        ingress
            .attachments
            .push(claw_core::channel_ingress::ChannelIngressAttachment {
                kind: kind.to_string(),
                path: format!("provider://wechat/{message_id}"),
                mime_type: None,
                size: None,
            });
    }
    let request = SubmitTaskRequest {
        user_id: None,
        chat_id: None,
        user_key: None,
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some(from_user_id.to_string()),
        external_chat_id: Some(scoped_chat_id.clone()),
        ingress: Some(ingress),
        idempotency_key: Some(idempotency_key.clone()),
        kind: TaskKind::Ask,
        payload: json!({
            "text": prompt,
            "context_token": task_context.context_token.clone(),
            "channel_account_id": task_context.account.account_id.clone(),
        }),
    };
    let response = state
        .client
        .post(format!(
            "{}/v1/auth/channel/pending-request",
            state.config.clawd_base_url.trim_end_matches('/')
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
        return Err(
            claw_core::channel_provider_error::ChannelProviderError::invalid_response(
                "wechat_ilink",
                "store_pending_request",
                body.error.as_deref().unwrap_or("application_rejected"),
            )
            .to_string(),
        );
    }
    Ok(body.data)
}
