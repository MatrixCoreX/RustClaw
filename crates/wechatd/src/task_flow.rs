use super::*;

const WECHAT_TASK_DONE_FALLBACK_TEXT_KEY: &str = "wechat.msg.task_done_fallback_text";
const WECHAT_TASK_FAILED_FALLBACK_ERROR_KEY: &str = "wechat.msg.task_failed_fallback_error";
const WECHAT_REQUEST_TIMEOUT_RETRY_LATER_KEY: &str = "wechat.msg.request_timeout_retry_later";

pub(super) fn task_success_messages(
    task: &TaskQueryResponse,
    config: &WechatSection,
) -> Vec<String> {
    if let Some(messages) = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
    {
        let parts: Vec<String> = messages
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if !parts.is_empty() {
            return parts;
        }
    }
    vec![task
        .result_json
        .as_ref()
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| wechat_t(config, WECHAT_TASK_DONE_FALLBACK_TEXT_KEY))]
}

/// Refresh `ilink/bot/sendtyping` while clawd runs (`keepaliveIntervalMs` ≈ 5s in OpenClaw weixin).
pub(super) struct WechatTypingHeartbeat {
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl WechatTypingHeartbeat {
    fn start(
        client: Client,
        section: WechatSection,
        base_url: String,
        token: String,
        to_user_id: String,
        typing_ticket: String,
        interval: Duration,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                let _ = ilink::send_typing_once(
                    &client,
                    &section,
                    &base_url,
                    &token,
                    &to_user_id,
                    &typing_ticket,
                    1,
                )
                .await;
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = &mut stop_rx => {
                        let _ = ilink::send_typing_once(
                            &client,
                            &section,
                            &base_url,
                            &token,
                            &to_user_id,
                            &typing_ticket,
                            2,
                        )
                        .await;
                        break;
                    }
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
        }
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for WechatTypingHeartbeat {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) async fn start_typing_heartbeat_for_peer(
    state: &State,
    from_user_id: &str,
    typing_ticket: Option<&str>,
) -> Option<WechatTypingHeartbeat> {
    let ticket = typing_ticket
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let session_guard = state.session.read().await;
    let token = session_token(&state.config, session_guard.as_ref())?;
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let interval = Duration::from_secs(state.config.typing_refresh_interval_secs.max(1));
    Some(WechatTypingHeartbeat::start(
        state.client.clone(),
        state.config.clone(),
        base_url,
        token.clone(),
        from_user_id.to_string(),
        ticket,
        interval,
    ))
}

pub(super) async fn resolve_typing_ticket_for_peer(
    state: &State,
    from_user_id: &str,
    context_token: Option<&str>,
) -> Option<String> {
    let context_token = resolve_delivery_context_token(state, from_user_id, context_token).await;
    let session_guard = state.session.read().await;
    let token = session_token(&state.config, session_guard.as_ref())?;
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let mut mgr = state.config_cache.lock().await;
    let ticket = mgr
        .typing_ticket_for_user(
            &state.client,
            &state.config,
            &base_url,
            &token,
            from_user_id,
            context_token.as_deref(),
        )
        .await;
    let t = ticket.trim();
    if t.is_empty() {
        None
    } else {
        Some(ticket)
    }
}

pub(super) async fn deliver_wechat_clawd_reply(
    state: &State,
    from_user_id: &str,
    context_token: Option<&str>,
    reply_text: &str,
) {
    let session_guard = state.session.read().await;
    let Some(token) = session_token(&state.config, session_guard.as_ref()) else {
        warn!("wechatd: deliver reply skipped (no session token)");
        return;
    };
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let Some(context_token) =
        resolve_delivery_context_token(state, from_user_id, context_token).await
    else {
        warn!("wechatd: deliver reply skipped (missing context_token)");
        return;
    };
    let timeout_ms = state.config.request_timeout_seconds.max(1) * 1_000;
    let cdn = state.config.cdn_base_url.trim();
    let auth = wechat_ilink_auth(&state.config);
    let media = extract_wechat_outbound_media(reply_text, &state.workspace_root);
    let stripped = markdown_to_plain_text(&strip_wechat_delivery_lines(reply_text));
    let no_outbound_media = media.is_empty();
    if !stripped.trim().is_empty() {
        if let Err(err) = send_text_message(
            &state.client,
            &state.config,
            &base_url,
            &token,
            from_user_id,
            Some(context_token.as_str()),
            stripped.trim(),
        )
        .await
        {
            warn!("wechatd: send reply text failed err={}", err);
        }
    }
    let mut media_error_notified = false;
    for media in &media {
        let file_path = match materialize_wechat_outbound_media(state, media).await {
            Ok(path) => path,
            Err(err) => {
                warn!(
                    "wechatd: prepare reply media {:?} kind={:?} err={}",
                    media, media.kind, err
                );
                if !media_error_notified {
                    send_wechat_error_notice(
                        state,
                        &base_url,
                        &token,
                        from_user_id,
                        context_token.as_str(),
                        &err,
                    )
                    .await;
                    media_error_notified = true;
                }
                continue;
            }
        };
        let res = match media.kind {
            WechatOutboundKind::Image => {
                send_weixin_image_from_file(
                    &state.client,
                    &base_url,
                    &token,
                    auth,
                    cdn,
                    from_user_id,
                    Some(context_token.as_str()),
                    &file_path,
                    WECHATD_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
            WechatOutboundKind::Video => {
                send_weixin_video_from_file(
                    &state.client,
                    &base_url,
                    &token,
                    auth,
                    cdn,
                    from_user_id,
                    Some(context_token.as_str()),
                    &file_path,
                    WECHATD_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
            WechatOutboundKind::File => {
                let fname = file_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file");
                send_weixin_file_from_file(
                    &state.client,
                    &base_url,
                    &token,
                    auth,
                    cdn,
                    from_user_id,
                    Some(context_token.as_str()),
                    &file_path,
                    fname,
                    WECHATD_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
        };
        if let Err(err) = res {
            warn!(
                "wechatd: send reply media {:?} kind={:?} err={}",
                file_path, media.kind, err
            );
            if !media_error_notified {
                send_wechat_error_notice(
                    state,
                    &base_url,
                    &token,
                    from_user_id,
                    context_token.as_str(),
                    &err,
                )
                .await;
                media_error_notified = true;
            }
        }
    }
    if stripped.trim().is_empty() && no_outbound_media && !reply_text.trim().is_empty() {
        let fallback_text = markdown_to_plain_text(reply_text);
        if let Err(err) = send_text_message(
            &state.client,
            &state.config,
            &base_url,
            &token,
            from_user_id,
            Some(context_token.as_str()),
            &fallback_text,
        )
        .await
        {
            warn!("wechatd: send reply fallback text failed err={}", err);
        }
    }
}

pub(super) async fn submit_wechat_task_with_payload(
    state: State,
    from_user_id: String,
    context_token: Option<String>,
    user_key: Option<String>,
    typing_ticket: Option<String>,
    kind: TaskKind,
    mut payload: Value,
) {
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("channel")
            .or_insert(Value::String("wechat".to_string()));
        if let Some(ref ct) = context_token {
            let t = ct.trim();
            if !t.is_empty() {
                obj.entry("context_token")
                    .or_insert(Value::String(ct.clone()));
            }
        }
    }
    let submit_req = SubmitTaskRequest {
        user_id: Some(stable_i64_from_string(&from_user_id)),
        chat_id: Some(stable_i64_from_string(&from_user_id)),
        user_key: user_key.clone(),
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some(from_user_id.clone()),
        external_chat_id: Some(from_user_id.clone()),
        kind,
        payload,
    };
    let submit_url = format!(
        "{}/v1/tasks",
        state.config.clawd_base_url.trim_end_matches('/')
    );
    let submit_resp = match state
        .client
        .post(&submit_url)
        .json(&submit_req)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            warn!("wechatd: task submit failed err={}", err);
            return;
        }
    };
    if !submit_resp.status().is_success() {
        warn!(
            "wechatd: task submit failed status={} body={}",
            submit_resp.status(),
            submit_resp.text().await.unwrap_or_default()
        );
        return;
    }
    let submit_body: ApiResponse<SubmitTaskResponse> = match submit_resp.json().await {
        Ok(body) => body,
        Err(err) => {
            warn!("wechatd: task submit parse failed err={}", err);
            return;
        }
    };
    let Some(task_data) = submit_body.data else {
        warn!("wechatd: task submit missing task_id");
        return;
    };
    let task_id = task_data.task_id.to_string();
    let started = std::time::Instant::now();
    let delivery_timeout_secs = state.config.task_delivery_timeout_seconds.max(1);
    let poll_interval = Duration::from_millis(1500);
    let running_notice_text = wechat_t_with(
        &state.config,
        WECHAT_REQUEST_TIMEOUT_RETRY_LATER_KEY,
        &[("task_id", &task_id)],
    );
    let mut timeout_notice_sent = false;
    let mut last_seen_status: Option<TaskStatus> = None;
    let (poll_token, poll_base) = {
        let g = state.session.read().await;
        (
            session_token(&state.config, g.as_ref()),
            session_base_url(&state.config, g.as_ref()),
        )
    };
    let interval = Duration::from_secs(state.config.typing_refresh_interval_secs.max(1));
    let _typing_guard = match (&typing_ticket, &poll_token) {
        (Some(ticket), Some(tok)) if !ticket.trim().is_empty() => {
            Some(WechatTypingHeartbeat::start(
                state.client.clone(),
                state.config.clone(),
                poll_base,
                tok.clone(),
                from_user_id.clone(),
                ticket.clone(),
                interval,
            ))
        }
        _ => None,
    };
    loop {
        let url = format!(
            "{}/v1/tasks/{}",
            state.config.clawd_base_url.trim_end_matches('/'),
            task_id
        );
        let mut req = state.client.get(&url);
        if let Some(ref key) = user_key {
            let k = key.trim();
            if !k.is_empty() {
                req = req.header("X-RustClaw-Key", k);
            }
        }
        let resp = match req.send().await {
            Ok(resp) => resp,
            Err(err) => {
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                    if !timeout_notice_sent {
                        warn!(
                            "wechatd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=poll_failed (continue_polling=true) err={}",
                            task_id,
                            started.elapsed().as_secs(),
                            delivery_timeout_secs,
                            last_seen_status,
                            err
                        );
                        send_text_reply_via_session(
                            &state,
                            &from_user_id,
                            context_token.as_deref(),
                            &running_notice_text,
                        )
                        .await;
                        timeout_notice_sent = true;
                    }
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };
        if !resp.status().is_success() {
            if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                if !timeout_notice_sent {
                    warn!(
                        "wechatd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=http_status (continue_polling=true) status={}",
                        task_id,
                        started.elapsed().as_secs(),
                        delivery_timeout_secs,
                        last_seen_status,
                        resp.status()
                    );
                    send_text_reply_via_session(
                        &state,
                        &from_user_id,
                        context_token.as_deref(),
                        &running_notice_text,
                    )
                    .await;
                    timeout_notice_sent = true;
                }
            }
            tokio::time::sleep(poll_interval).await;
            continue;
        }
        let body: ApiResponse<TaskQueryResponse> = match resp.json().await {
            Ok(body) => body,
            Err(err) => {
                warn!("wechatd: poll task parse failed err={}", err);
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                    if !timeout_notice_sent {
                        warn!(
                            "wechatd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=parse_failed (continue_polling=true)",
                            task_id,
                            started.elapsed().as_secs(),
                            delivery_timeout_secs,
                            last_seen_status
                        );
                        send_text_reply_via_session(
                            &state,
                            &from_user_id,
                            context_token.as_deref(),
                            &running_notice_text,
                        )
                        .await;
                        timeout_notice_sent = true;
                    }
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };
        let Some(task) = body.data else {
            if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                if !timeout_notice_sent {
                    warn!(
                        "wechatd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=no_task_data (continue_polling=true)",
                        task_id,
                        started.elapsed().as_secs(),
                        delivery_timeout_secs,
                        last_seen_status
                    );
                    send_text_reply_via_session(
                        &state,
                        &from_user_id,
                        context_token.as_deref(),
                        &running_notice_text,
                    )
                    .await;
                    timeout_notice_sent = true;
                }
            }
            tokio::time::sleep(poll_interval).await;
            continue;
        };
        last_seen_status = Some(task.status.clone());
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                    if !timeout_notice_sent {
                        warn!(
                            "wechatd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} (continue_polling=true)",
                            task_id,
                            started.elapsed().as_secs(),
                            delivery_timeout_secs,
                            last_seen_status
                        );
                        send_text_reply_via_session(
                            &state,
                            &from_user_id,
                            context_token.as_deref(),
                            &running_notice_text,
                        )
                        .await;
                        timeout_notice_sent = true;
                    }
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
            TaskStatus::Succeeded => {
                for reply_text in task_success_messages(&task, &state.config) {
                    deliver_wechat_clawd_reply(
                        &state,
                        &from_user_id,
                        context_token.as_deref(),
                        &reply_text,
                    )
                    .await;
                }
                break;
            }
            TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                let error_text = task.error_text.unwrap_or_else(|| {
                    wechat_t(&state.config, WECHAT_TASK_FAILED_FALLBACK_ERROR_KEY)
                });
                send_text_reply_via_session(
                    &state,
                    &from_user_id,
                    context_token.as_deref(),
                    &error_text,
                )
                .await;
                break;
            }
        }
    }
}

pub(super) async fn submit_wechat_task_and_reply(
    state: State,
    from_user_id: String,
    text: String,
    context_token: Option<String>,
    user_key: Option<String>,
    typing_ticket: Option<String>,
) {
    let payload = json!({
        "text": text,
        "channel": "wechat",
        "context_token": context_token.clone(),
    });
    submit_wechat_task_with_payload(
        state,
        from_user_id,
        context_token,
        user_key,
        typing_ticket,
        TaskKind::Ask,
        payload,
    )
    .await;
}

pub(super) async fn submit_wechat_run_skill_and_reply(
    state: State,
    from_user_id: String,
    context_token: Option<String>,
    user_key: Option<String>,
    typing_ticket: Option<String>,
    skill_name: &'static str,
    args: Value,
) {
    let payload = json!({
        "skill_name": skill_name,
        "args": args,
    });
    submit_wechat_task_with_payload(
        state,
        from_user_id,
        context_token,
        user_key,
        typing_ticket,
        TaskKind::RunSkill,
        payload,
    )
    .await;
}

pub(super) async fn spawn_inbound_ask_flow(
    state: State,
    from_user_id: String,
    msg: WeixinMessage,
    ask_text: String,
    user_key: String,
    prefetched_typing_ticket: Option<String>,
) {
    let typing_ticket = prefetched_typing_ticket.filter(|ticket| !ticket.trim().is_empty());
    tokio::spawn(submit_wechat_task_and_reply(
        state,
        from_user_id,
        ask_text,
        msg.context_token,
        Some(user_key),
        typing_ticket,
    ));
}

pub(super) async fn spawn_inbound_skill_flow(
    state: State,
    from_user_id: String,
    msg: WeixinMessage,
    skill_name: &'static str,
    args: Value,
    user_key: String,
    prefetched_typing_ticket: Option<String>,
) {
    let typing_ticket = prefetched_typing_ticket.filter(|ticket| !ticket.trim().is_empty());
    tokio::spawn(submit_wechat_run_skill_and_reply(
        state,
        from_user_id,
        msg.context_token,
        Some(user_key),
        typing_ticket,
        skill_name,
        args,
    ));
}
