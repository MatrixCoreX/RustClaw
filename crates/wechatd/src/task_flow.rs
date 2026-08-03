use super::*;

const WECHAT_TASK_FAILED_FALLBACK_ERROR_KEY: &str = "wechat.msg.task_failed_fallback_error";
const WECHAT_REQUEST_TIMEOUT_RETRY_LATER_KEY: &str = "wechat.msg.request_timeout_retry_later";
const WECHAT_SKILL_PROGRESS_KB_KEY: &str = "wechat.msg.skill_progress_kb";
const WECHAT_SKILL_PROGRESS_PACKAGE_KEY: &str = "wechat.msg.skill_progress_package";
const WECHAT_SKILL_PROGRESS_GENERIC_KEY: &str = "wechat.msg.skill_progress_generic";

#[derive(Clone)]
pub(super) struct WechatAccountSnapshot {
    pub(super) account_id: String,
    pub(super) base_url: String,
    pub(super) token: String,
}

#[derive(Clone)]
pub(super) struct PinnedWechatTaskContext {
    pub(super) account: WechatAccountSnapshot,
    pub(super) scope: WechatConversationScope,
    pub(super) context_token: String,
    pub(super) typing_ticket: Option<String>,
    pub(super) run_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WechatTaskTerminalKind {
    Succeeded,
    Failed,
    Canceled,
    Timeout,
}

pub(super) fn wechat_task_terminal_kind(status: TaskStatus) -> Option<WechatTaskTerminalKind> {
    match status {
        TaskStatus::Queued | TaskStatus::Running => None,
        TaskStatus::Succeeded => Some(WechatTaskTerminalKind::Succeeded),
        TaskStatus::Failed => Some(WechatTaskTerminalKind::Failed),
        TaskStatus::Canceled => Some(WechatTaskTerminalKind::Canceled),
        TaskStatus::Timeout => Some(WechatTaskTerminalKind::Timeout),
    }
}

pub(super) async fn pin_inbound_task_context(
    state: &State,
    peer_id: &str,
    inbound_context_token: Option<&str>,
) -> Option<PinnedWechatTaskContext> {
    // The provider issues this token per inbound message. A task must never
    // borrow a later or older token from the cache when the inbound token is
    // absent.
    let context_token = normalized_context_token(inbound_context_token)?.to_string();
    let account = {
        let session = state.session.read().await;
        WechatAccountSnapshot {
            account_id: session_account_id(session.as_ref()),
            base_url: session_base_url(&state.config, session.as_ref()),
            token: session_token(&state.config, session.as_ref())?,
        }
    };
    let scope = WechatConversationScope::wechat_ilink(&account.account_id, peer_id).ok()?;
    state
        .context_tokens
        .write()
        .await
        .insert(scope.storage_key(), context_token.clone());
    let typing_ticket = {
        let mut manager = state.config_cache.lock().await;
        manager
            .typing_ticket_for_user(
                &state.client,
                &state.config,
                &account.base_url,
                &account.token,
                &scope,
                peer_id,
                Some(&context_token),
            )
            .await
    };
    Some(PinnedWechatTaskContext {
        account,
        scope,
        context_token,
        typing_ticket: (!typing_ticket.trim().is_empty()).then_some(typing_ticket),
        run_id: wechat_ilink::new_wechat_client_id("run"),
    })
}

pub(super) fn skill_progress_message(
    task: &TaskQueryResponse,
    config: &WechatSection,
) -> Option<(u64, String)> {
    let event = task.skill_progress.as_ref()?;
    let seq = event.get("seq")?.as_u64()?;
    let payload = event.get("payload")?;
    if payload.get("source").and_then(Value::as_str) != Some("skill_progress")
        || payload.get("data_only").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let detail_key = payload
        .pointer("/frame/detail_key")
        .and_then(Value::as_str)?;
    let message_key = match detail_key {
        // Media stages may only project a model-authored task-plan title. The
        // frame itself remains machine data and is never translated here.
        detail_key if detail_key.starts_with("media_download.") => {
            let step_id = payload
                .pointer("/frame/params/step_id")
                .and_then(Value::as_str)?;
            return model_authored_plan_step_title(task, step_id).map(|title| (seq, title));
        }
        "skill_dispatch.queue.started" | "skill_dispatch.queue.waiting" => return None,
        "kb.operation.starting" => WECHAT_SKILL_PROGRESS_KB_KEY,
        "package_manager.operation.starting" => WECHAT_SKILL_PROGRESS_PACKAGE_KEY,
        _ => WECHAT_SKILL_PROGRESS_GENERIC_KEY,
    };
    Some((seq, wechat_t(config, message_key)))
}

fn model_authored_plan_step_title(task: &TaskQueryResponse, step_id: &str) -> Option<String> {
    let plan = task.task_plan.as_ref()?;
    if plan.get("source").and_then(Value::as_str) != Some("task_plan")
        || plan.get("status").and_then(Value::as_str) != Some("ok")
        || plan.get("data_only").and_then(Value::as_bool) != Some(true)
        || plan.get("render_owner").and_then(Value::as_str) != Some("ui_cli_channel_projection")
    {
        return None;
    }
    let title = plan
        .get("steps")?
        .as_array()?
        .iter()
        .find(|step| step.get("step_id").and_then(Value::as_str) == Some(step_id))?
        .get("title")?
        .as_str()?
        .trim();
    (!title.is_empty() && title.chars().count() <= 512).then(|| title.to_string())
}

/// Refresh `ilink/bot/sendtyping` while clawd runs (`keepaliveIntervalMs` ≈ 5s in OpenClaw weixin).
pub(super) struct WechatTypingHeartbeat {
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl WechatTypingHeartbeat {
    pub(super) fn start(
        client: Client,
        section: WechatSection,
        base_url: String,
        token: String,
        to_user_id: String,
        typing_ticket: String,
        interval: Duration,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let join_handle = tokio::spawn(async move {
            loop {
                let _ = ilink::send_typing_once(
                    &client,
                    &section,
                    &base_url,
                    &token,
                    &to_user_id,
                    &typing_ticket,
                    TYPING_STATUS_TYPING,
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
                            TYPING_STATUS_CANCEL,
                        )
                        .await;
                        break;
                    }
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
            join_handle: Some(join_handle),
        }
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }

    pub(super) async fn finish(&mut self) {
        self.stop();
        let Some(mut handle) = self.join_handle.take() else {
            return;
        };
        if tokio::time::timeout(Duration::from_secs(10), &mut handle)
            .await
            .is_err()
        {
            handle.abort();
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
    context: &PinnedWechatTaskContext,
) -> Option<WechatTypingHeartbeat> {
    let ticket = context
        .typing_ticket
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let interval = Duration::from_secs(state.config.typing_refresh_interval_secs.max(1));
    Some(WechatTypingHeartbeat::start(
        state.client.clone(),
        state.config.clone(),
        context.account.base_url.clone(),
        context.account.token.clone(),
        context.scope.peer_id().to_string(),
        ticket,
        interval,
    ))
}

pub(super) async fn send_generating_message_state(
    state: &State,
    context: &PinnedWechatTaskContext,
) -> Result<(), String> {
    let body = WechatSendMessageRequest::generating(
        context.scope.peer_id(),
        &context.context_token,
        wechat_ilink::new_wechat_client_id("generating"),
        context.run_id.clone(),
        WECHATD_CHANNEL_VERSION,
    )?;
    ilink::post_json(
        &state.client,
        &state.config,
        &context.account.base_url,
        &context.account.token,
        "ilink/bot/sendmessage",
        &body,
        state.config.request_timeout_seconds.max(1) * 1_000,
    )
    .await
    .map(|_| ())
}

async fn finish_typing_heartbeat(heartbeat: &mut Option<WechatTypingHeartbeat>) {
    if let Some(heartbeat) = heartbeat.as_mut() {
        heartbeat.finish().await;
    }
    *heartbeat = None;
}

async fn deliver_pinned_terminal_text(
    state: &State,
    context: &PinnedWechatTaskContext,
    text: &str,
) {
    if let Err(error) = send_text_message(
        &state.client,
        &state.config,
        &context.account.base_url,
        &context.account.token,
        context.scope.peer_id(),
        Some(&context.context_token),
        Some(&context.run_id),
        text,
    )
    .await
    {
        warn!("wechatd: terminal text delivery failed err={}", error);
    }
}

async fn deliver_pinned_progress_text(
    state: &State,
    context: &PinnedWechatTaskContext,
    text: &str,
) {
    let body = match WechatSendMessageRequest::generating_with_item(
        context.scope.peer_id(),
        &context.context_token,
        wechat_ilink::new_wechat_client_id("progress"),
        context.run_id.clone(),
        match WechatMessageItem::text(text) {
            Ok(item) => item,
            Err(error) => {
                warn!("wechatd: progress item rejected err={}", error);
                return;
            }
        },
        WECHATD_CHANNEL_VERSION,
    ) {
        Ok(body) => body,
        Err(error) => {
            warn!("wechatd: progress envelope rejected err={}", error);
            return;
        }
    };
    if let Err(error) = ilink::post_json(
        &state.client,
        &state.config,
        &context.account.base_url,
        &context.account.token,
        "ilink/bot/sendmessage",
        &body,
        state.config.request_timeout_seconds.max(1) * 1_000,
    )
    .await
    {
        warn!("wechatd: progress delivery failed err={}", error);
    }
}

async fn finalize_submit_failure(
    state: &State,
    context: &PinnedWechatTaskContext,
    heartbeat: &mut Option<WechatTypingHeartbeat>,
) {
    finish_typing_heartbeat(heartbeat).await;
    let text = wechat_t(&state.config, WECHAT_TASK_FAILED_FALLBACK_ERROR_KEY);
    deliver_pinned_terminal_text(state, context, &text).await;
}

pub(super) async fn submit_wechat_task_with_payload(
    state: State,
    context: PinnedWechatTaskContext,
    user_key: Option<String>,
    kind: TaskKind,
    mut payload: Value,
    existing_task_id: Option<String>,
) {
    let from_user_id = context.scope.peer_id().to_string();
    let context_token = context.context_token.clone();
    let scoped_chat_id = context.scope.storage_key();
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("channel")
            .or_insert(Value::String("wechat".to_string()));
        obj.insert(
            "context_token".to_string(),
            Value::String(context_token.clone()),
        );
        obj.insert(
            "channel_account_id".to_string(),
            Value::String(context.account.account_id.clone()),
        );
    }
    let submit_req = SubmitTaskRequest {
        user_id: Some(stable_i64_from_string(&from_user_id)),
        chat_id: Some(stable_i64_from_string(&scoped_chat_id)),
        user_key: user_key.clone(),
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some(from_user_id.clone()),
        external_chat_id: Some(scoped_chat_id.clone()),
        ingress: Some({
            claw_core::channel_ingress::ChannelIngressEnvelope::new(
                ChannelKind::Wechat,
                "wechat_ilink",
            )
            .with_external_ids(from_user_id.clone(), scoped_chat_id)
            .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::user(
                from_user_id.clone(),
            ))
            .with_locale(state.config.language.clone())
            .with_context_token(context_token.clone())
        }),
        idempotency_key: None,
        kind,
        payload,
    };
    let mut typing_heartbeat = start_typing_heartbeat_for_peer(&state, &context).await;
    if let Err(error) = send_generating_message_state(&state, &context).await {
        warn!("wechatd: generating state delivery failed err={}", error);
    }
    let submit_url = format!(
        "{}/v1/tasks",
        state.config.clawd_base_url.trim_end_matches('/')
    );
    let task_id = if let Some(task_id) = existing_task_id {
        task_id
    } else {
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
                finalize_submit_failure(&state, &context, &mut typing_heartbeat).await;
                return;
            }
        };
        if !submit_resp.status().is_success() {
            let status = submit_resp.status();
            let body = submit_resp.text().await.unwrap_or_default();
            let error = claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                "wechat_ilink",
                "submit_task",
                status.as_u16(),
                &body,
            );
            warn!(
                "wechatd: task submit failed error_code={} diagnostic_id={}",
                error.error_code, error.diagnostic_id
            );
            finalize_submit_failure(&state, &context, &mut typing_heartbeat).await;
            return;
        }
        let submit_body: ApiResponse<SubmitTaskResponse> = match submit_resp.json().await {
            Ok(body) => body,
            Err(err) => {
                warn!("wechatd: task submit parse failed err={}", err);
                finalize_submit_failure(&state, &context, &mut typing_heartbeat).await;
                return;
            }
        };
        let Some(task_data) = submit_body.data else {
            warn!("wechatd: task submit missing task_id");
            finalize_submit_failure(&state, &context, &mut typing_heartbeat).await;
            return;
        };
        task_data.task_id.to_string()
    };
    let started = std::time::Instant::now();
    let delivery_timeout_secs = state.config.task_delivery_timeout_seconds.max(1);
    let poll_interval = Duration::from_millis(1500);
    let running_notice_text = wechat_t_with(
        &state.config,
        WECHAT_REQUEST_TIMEOUT_RETRY_LATER_KEY,
        &[("task_id", &task_id)],
    );
    let mut timeout_notice_sent = false;
    let mut last_skill_progress_seq = 0_u64;
    let mut last_seen_status: Option<TaskStatus> = None;
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
                req = req.header("X-Agent-Key", k);
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
                        deliver_pinned_progress_text(&state, &context, &running_notice_text).await;
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
                    deliver_pinned_progress_text(&state, &context, &running_notice_text).await;
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
                        deliver_pinned_progress_text(&state, &context, &running_notice_text).await;
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
                    deliver_pinned_progress_text(&state, &context, &running_notice_text).await;
                    timeout_notice_sent = true;
                }
            }
            tokio::time::sleep(poll_interval).await;
            continue;
        };
        last_seen_status = Some(task.status.clone());
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                if let Some((seq, message)) = skill_progress_message(&task, &state.config) {
                    if seq > last_skill_progress_seq {
                        deliver_pinned_progress_text(&state, &context, &message).await;
                        last_skill_progress_seq = seq;
                    }
                }
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                    if !timeout_notice_sent {
                        warn!(
                            "wechatd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} (continue_polling=true)",
                            task_id,
                            started.elapsed().as_secs(),
                            delivery_timeout_secs,
                            last_seen_status
                        );
                        deliver_pinned_progress_text(&state, &context, &running_notice_text).await;
                        timeout_notice_sent = true;
                    }
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
            terminal_status @ (TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Timeout) => {
                debug_assert!(wechat_task_terminal_kind(terminal_status).is_some());
                finish_typing_heartbeat(&mut typing_heartbeat).await;
                request_unified_terminal_delivery(
                    &state,
                    &task_id,
                    user_key.as_deref(),
                    timeout_notice_sent,
                )
                .await;
                break;
            }
        }
    }
}

async fn request_unified_terminal_delivery(
    state: &State,
    task_id: &str,
    user_key: Option<&str>,
    background: bool,
) {
    let Some(user_key) = user_key.map(str::trim).filter(|value| !value.is_empty()) else {
        warn!("wechatd: terminal delivery missing bound key task_id={task_id}");
        return;
    };
    let source = if background {
        claw_core::channel_delivery::ChannelDeliverySource::BackgroundCompletion
    } else {
        claw_core::channel_delivery::ChannelDeliverySource::ImmediateDaemon
    };
    match claw_core::channel_delivery_client::request_task_delivery(
        &state.client,
        &state.config.clawd_base_url,
        task_id,
        user_key,
        source,
    )
    .await
    {
        Ok(result) if result.accepted => info!(
            "wechatd: unified terminal delivery accepted task_id={} status={:?}",
            task_id, result.status
        ),
        Ok(result) => warn!(
            "wechatd: unified terminal delivery not accepted task_id={} status={:?} error_code={}",
            task_id,
            result.status,
            result.error_code.as_deref().unwrap_or("none")
        ),
        Err(error) => warn!(
            "wechatd: unified terminal delivery request failed task_id={} error={}",
            task_id, error
        ),
    }
}

pub(super) async fn submit_wechat_task_and_reply(
    state: State,
    context: PinnedWechatTaskContext,
    text: String,
    user_key: Option<String>,
) {
    let payload = json!({
        "text": text,
        "channel": "wechat",
        "context_token": context.context_token.clone(),
    });
    submit_wechat_task_with_payload(state, context, user_key, TaskKind::Ask, payload, None).await;
}

pub(super) async fn submit_wechat_run_skill_and_reply(
    state: State,
    context: PinnedWechatTaskContext,
    user_key: Option<String>,
    skill_name: &'static str,
    args: Value,
) {
    let payload = json!({
        "skill_name": skill_name,
        "args": args,
    });
    submit_wechat_task_with_payload(state, context, user_key, TaskKind::RunSkill, payload, None)
        .await;
}

pub(super) async fn spawn_existing_wechat_task_delivery(
    state: State,
    context: PinnedWechatTaskContext,
    user_key: String,
    task_id: String,
) {
    tokio::spawn(submit_wechat_task_with_payload(
        state,
        context,
        Some(user_key),
        TaskKind::Ask,
        json!({}),
        Some(task_id),
    ));
}

pub(super) async fn spawn_inbound_ask_flow(
    state: State,
    context: PinnedWechatTaskContext,
    ask_text: String,
    user_key: String,
) {
    tokio::spawn(submit_wechat_task_and_reply(
        state,
        context,
        ask_text,
        Some(user_key),
    ));
}

pub(super) async fn spawn_inbound_skill_flow(
    state: State,
    context: PinnedWechatTaskContext,
    skill_name: &'static str,
    args: Value,
    user_key: String,
) {
    tokio::spawn(submit_wechat_run_skill_and_reply(
        state,
        context,
        Some(user_key),
        skill_name,
        args,
    ));
}
