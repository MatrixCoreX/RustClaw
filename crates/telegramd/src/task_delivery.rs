use super::*;

struct TypingHeartbeatGuard {
    stop_tx: Option<oneshot::Sender<()>>,
}

impl TypingHeartbeatGuard {
    fn start(bot: Bot, chat_id: ChatId) -> Self {
        const REFRESH_INTERVAL: Duration = Duration::from_secs(4);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        tokio::spawn(async move {
            loop {
                let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                tokio::select! {
                    _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
                    _ = &mut stop_rx => break,
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
        }
    }
}

impl Drop for TypingHeartbeatGuard {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
    }
}

pub(super) fn spawn_task_result_delivery(
    bot: Bot,
    state: BotState,
    chat_id: ChatId,
    user_id: i64,
    task_id: String,
    soft_notice_override_seconds: Option<u64>,
    fail_prefix: String,
) {
    spawn_task_result_delivery_with_mode(
        bot,
        state,
        chat_id,
        user_id,
        task_id,
        soft_notice_override_seconds,
        fail_prefix,
        false,
    );
}

pub(super) fn spawn_voice_task_result_delivery(
    bot: Bot,
    state: BotState,
    chat_id: ChatId,
    user_id: i64,
    task_id: String,
    fail_prefix: String,
) {
    spawn_task_result_delivery_with_mode(
        bot,
        state,
        chat_id,
        user_id,
        task_id,
        None,
        fail_prefix,
        true,
    );
}

fn spawn_task_result_delivery_with_mode(
    bot: Bot,
    state: BotState,
    chat_id: ChatId,
    user_id: i64,
    task_id: String,
    soft_notice_override_seconds: Option<u64>,
    fail_prefix: String,
    voice_reply: bool,
) {
    tokio::spawn(async move {
        let _typing_guard = TypingHeartbeatGuard::start(bot.clone(), chat_id);
        let poll_interval_ms = state.poll_interval_ms.max(1);
        // 0 表示不发送“任务已运行超过 X 秒”的提示
        let soft_notice_seconds = soft_notice_override_seconds.unwrap_or(state.task_wait_seconds);
        let hard_notice_seconds = state.task_wait_seconds;
        let started_at = tokio::time::Instant::now();
        let mut soft_notice_sent = false;
        let mut hard_notice_sent = false;
        let mut sent_progress_count = 0usize;
        let mut last_skill_progress_seq = 0_u64;

        loop {
            match query_task_status(
                &state,
                &task_id,
                bound_user_key_for_chat(&state, chat_id.0).as_deref(),
            )
            .await
            {
                Ok(task) => match task.status {
                    TaskStatus::Queued | TaskStatus::Running => {
                        if let Some((seq, message)) = skill_progress_message(&state, &task) {
                            if seq > last_skill_progress_seq {
                                let _ = bot.send_message(chat_id, message).await;
                                last_skill_progress_seq = seq;
                                sent_progress_count = sent_progress_count.saturating_add(1);
                            }
                        }
                        let progress_messages = task_progress_messages(&task);
                        debug!(
                            "phase=poll task_id={} chat_id={} status={:?} elapsed_ms={} sent_progress_count={} progress_len={}",
                            task_id,
                            chat_id.0,
                            task.status,
                            started_at.elapsed().as_millis(),
                            sent_progress_count,
                            progress_messages.len()
                        );
                        if sent_progress_count < progress_messages.len() {
                            debug!(
                                "phase=skip_progress_delivery task_id={} chat_id={} skipped_count={}",
                                task_id,
                                chat_id.0,
                                progress_messages.len() - sent_progress_count
                            );
                            sent_progress_count = progress_messages.len();
                        }
                        if soft_notice_seconds > 0
                            && !soft_notice_sent
                            && started_at.elapsed() >= Duration::from_secs(soft_notice_seconds)
                        {
                            info!(
                                "task still running notice: phase=quick task_id={} chat_id={} elapsed_seconds={}",
                                task_id,
                                chat_id.0,
                                soft_notice_seconds
                            );
                            let soft_seconds = soft_notice_seconds.to_string();
                            let msg = state.i18n.t_with(
                                "telegram.msg.task_still_running_background",
                                &[
                                    ("seconds", soft_seconds.as_str()),
                                    ("task_id", task_id.as_str()),
                                ],
                            );
                            let _ = bot.send_message(chat_id, msg).await;
                            soft_notice_sent = true;
                        }
                        if hard_notice_seconds > 0
                            && !hard_notice_sent
                            && hard_notice_seconds > soft_notice_seconds
                            && started_at.elapsed() >= Duration::from_secs(hard_notice_seconds)
                        {
                            info!(
                                "task still running notice: phase=worker_timeout task_id={} chat_id={} elapsed_seconds={}",
                                task_id,
                                chat_id.0,
                                hard_notice_seconds
                            );
                            let hard_seconds = hard_notice_seconds.to_string();
                            let msg = state.i18n.t_with(
                                "telegram.msg.task_still_running_worker_timeout",
                                &[
                                    ("seconds", hard_seconds.as_str()),
                                    ("task_id", task_id.as_str()),
                                ],
                            );
                            let _ = bot.send_message(chat_id, msg).await;
                            hard_notice_sent = true;
                        }
                        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                    }
                    TaskStatus::Succeeded => {
                        let answers = task_success_messages(&state, &task);
                        let resume_followup_decision = task
                            .result_json
                            .as_ref()
                            .and_then(|v| v.get("resume_followup_decision"))
                            .and_then(|v| v.get("decision"))
                            .and_then(|v| v.as_str());
                        let has_structured_messages = task
                            .result_json
                            .as_ref()
                            .and_then(|v| v.get("messages"))
                            .and_then(|v| v.as_array())
                            .map(|arr| !arr.is_empty())
                            .unwrap_or(false);
                        if resume_followup_decision == Some("abandon") {
                            clear_pending_resume_for_chat(&state, chat_id.0);
                        } else if sent_progress_count > 0 || has_structured_messages {
                            clear_pending_resume_for_chat(&state, chat_id.0);
                        }
                        debug!(
                            "phase=deliver_success task_id={} chat_id={} sent_progress_count={} success_count={}",
                            task_id,
                            chat_id.0,
                            sent_progress_count,
                            answers.len(),
                        );
                        if voice_reply {
                            deliver_voice_answers(
                                &state,
                                chat_id,
                                user_id,
                                &task_id,
                                &answers,
                                soft_notice_sent || hard_notice_sent,
                            )
                            .await;
                        } else {
                            request_terminal_delivery(
                                &state,
                                chat_id.0,
                                &task_id,
                                soft_notice_sent || hard_notice_sent,
                            )
                            .await;
                        }
                        break;
                    }
                    TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                        request_terminal_delivery(
                            &state,
                            chat_id.0,
                            &task_id,
                            soft_notice_sent || hard_notice_sent,
                        )
                        .await;
                        if let Some(resume_context) = task
                            .result_json
                            .as_ref()
                            .and_then(|v| v.get("resume_context"))
                            .cloned()
                        {
                            let pending = PendingResumeContext {
                                user_id,
                                created_at_secs: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                                resume_context,
                            };
                            if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
                                guard.insert(chat_id.0, pending);
                            }
                            break;
                        }
                        break;
                    }
                },
                Err(err) => {
                    let _ = bot
                        .send_message(chat_id, format!("{fail_prefix}：{}", err))
                        .await;
                    break;
                }
            }
        }
    });
}

async fn deliver_voice_answers(
    state: &BotState,
    chat_id: ChatId,
    user_id: i64,
    original_task_id: &str,
    answers: &[String],
    background: bool,
) {
    let mode = parse_voice_reply_mode(&effective_voice_reply_mode_for_chat(state, chat_id.0));
    let original_content = if matches!(mode, VoiceReplyMode::Voice) {
        claw_core::channel_delivery::ChannelTaskDeliveryContent::MediaOnly
    } else {
        claw_core::channel_delivery::ChannelTaskDeliveryContent::Full
    };
    request_terminal_delivery_with_content(
        state,
        chat_id.0,
        original_task_id,
        background,
        original_content,
    )
    .await;
    if !matches!(mode, VoiceReplyMode::Voice | VoiceReplyMode::Both) {
        return;
    }

    let tts_input = terminal_tts_text(&answers.join("\n\n"));
    if tts_input.is_empty() {
        if matches!(mode, VoiceReplyMode::Voice) {
            request_terminal_delivery_with_content(
                state,
                chat_id.0,
                original_task_id,
                background,
                claw_core::channel_delivery::ChannelTaskDeliveryContent::TextOnly,
            )
            .await;
        }
        return;
    }
    let payload = json!({
        "skill_name": "audio_synthesize",
        "args": {"text": tts_input, "response_format": "opus"}
    });
    let Ok(task_id) =
        submit_task_only(state, user_id, chat_id.0, None, TaskKind::RunSkill, payload).await
    else {
        if matches!(mode, VoiceReplyMode::Voice) {
            request_terminal_delivery_with_content(
                state,
                chat_id.0,
                original_task_id,
                background,
                claw_core::channel_delivery::ChannelTaskDeliveryContent::TextOnly,
            )
            .await;
        }
        return;
    };
    match poll_task_result(
        state,
        &task_id,
        bound_user_key_for_chat(state, chat_id.0).as_deref(),
        Some(90),
    )
    .await
    {
        Ok(_) => {
            request_terminal_delivery(state, chat_id.0, &task_id, true).await;
        }
        Err(err) => {
            warn!("telegram voice reply synthesis failed: {err}");
            if matches!(mode, VoiceReplyMode::Voice) {
                request_terminal_delivery_with_content(
                    state,
                    chat_id.0,
                    original_task_id,
                    background,
                    claw_core::channel_delivery::ChannelTaskDeliveryContent::TextOnly,
                )
                .await;
            }
        }
    }
}

async fn request_terminal_delivery(
    state: &BotState,
    chat_id: i64,
    task_id: &str,
    background: bool,
) {
    request_terminal_delivery_with_content(
        state,
        chat_id,
        task_id,
        background,
        claw_core::channel_delivery::ChannelTaskDeliveryContent::Full,
    )
    .await;
}

async fn request_terminal_delivery_with_content(
    state: &BotState,
    chat_id: i64,
    task_id: &str,
    background: bool,
    content: claw_core::channel_delivery::ChannelTaskDeliveryContent,
) {
    let Some(user_key) = bound_user_key_for_chat(state, chat_id) else {
        warn!("telegramd: terminal delivery missing bound key task_id={task_id}");
        return;
    };
    let source = if background {
        claw_core::channel_delivery::ChannelDeliverySource::BackgroundCompletion
    } else {
        claw_core::channel_delivery::ChannelDeliverySource::ImmediateDaemon
    };
    match claw_core::channel_delivery_client::request_task_delivery_with_content(
        &state.client,
        &state.clawd_base_url,
        task_id,
        &user_key,
        source,
        content,
    )
    .await
    {
        Ok(result) if result.accepted => {
            info!(
                "telegramd: unified terminal delivery accepted task_id={} status={:?}",
                task_id, result.status
            );
        }
        Ok(result) => warn!(
            "telegramd: unified terminal delivery not accepted task_id={} status={:?} error_code={}",
            task_id,
            result.status,
            result.error_code.as_deref().unwrap_or("none")
        ),
        Err(error) => warn!(
            "telegramd: unified terminal delivery request failed task_id={} error={}",
            task_id, error
        ),
    }
}

pub(super) fn task_success_messages(state: &BotState, task: &TaskQueryResponse) -> Vec<String> {
    claw_core::task_delivery_artifacts::merge_task_artifact_delivery_messages(
        &task.task_id.to_string(),
        task.result_json.as_ref(),
        &state.workspace_root,
        task_success_messages_from_offset(state, task, 0),
    )
}

pub(super) fn task_success_messages_from_offset(
    state: &BotState,
    task: &TaskQueryResponse,
    offset: usize,
) -> Vec<String> {
    let task_id = &task.task_id;
    if let Some(messages) = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
    {
        let out = messages
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let out = dedupe_terminal_messages(out);
        if !out.is_empty() {
            debug!(
                "phase=success_source task_id={} source=messages offset={} messages_len={}",
                task_id,
                offset,
                out.len(),
            );
            if offset >= out.len() {
                // Progress delivery already consumed all message items.
                // Do not fallback to result_json.text here, otherwise the
                // last item is sent again (duplicate delivery).
                return Vec::new();
            }
            return out.into_iter().skip(offset).collect();
        }
    }
    let text = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.i18n.t("telegram.msg.task_done_no_text"));
    // Keep ask/run_skill success output as plain text to unify delivery format.
    let text = text;
    debug!(
        "phase=success_source task_id={} source=text_only offset={} text_fp={} text_len={}",
        task_id,
        offset,
        terminal_text_fingerprint_hex(&text),
        text.len()
    );
    vec![text]
}

pub(super) fn task_progress_messages(task: &TaskQueryResponse) -> Vec<String> {
    let out = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("progress_messages"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    dedupe_terminal_messages(out)
}

pub(super) fn skill_progress_message(
    state: &BotState,
    task: &TaskQueryResponse,
) -> Option<(u64, String)> {
    let event = task.skill_progress.as_ref()?;
    let seq = event.get("seq")?.as_u64()?;
    let payload = event.get("payload")?;
    if payload.get("source").and_then(serde_json::Value::as_str) != Some("skill_progress")
        || payload
            .get("data_only")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return None;
    }
    let detail_key = payload
        .pointer("/frame/detail_key")
        .and_then(serde_json::Value::as_str)?;
    let message_key = match detail_key {
        "media_download.precheck.starting" => "telegram.progress.skill_media_precheck",
        "kb.operation.starting" => "telegram.progress.skill_kb",
        "package_manager.operation.starting" => "telegram.progress.skill_package",
        _ => "telegram.progress.skill_generic",
    };
    Some((seq, state.i18n.t(message_key)))
}

pub(super) fn task_terminal_error_text(state: &BotState, task: &TaskQueryResponse) -> String {
    if let Some(raw_detail) = task.error_text.as_deref() {
        let detail = raw_detail.trim();
        if !detail.is_empty() {
            return detail.to_string();
        }
    }
    state.i18n.t_with(
        "telegram.error.task_finished_with_detail",
        &[
            ("status", &format!("{:?}", task.status)),
            (
                "detail",
                &task
                    .error_text
                    .clone()
                    .unwrap_or_else(|| state.i18n.t("telegram.msg.no_error_text")),
            ),
        ],
    )
}

pub(super) async fn query_task_status(
    state: &BotState,
    task_id: &str,
    user_key: Option<&str>,
) -> anyhow::Result<TaskQueryResponse> {
    let url = format!("{}/v1/tasks/{task_id}", state.clawd_base_url);
    let resp = maybe_with_user_key_header(state.client.get(&url), user_key)
        .send()
        .await
        .context("query task status failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let error = telegram_provider_http_error("query_task", status, &body);
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let body: ApiResponse<TaskQueryResponse> = resp
        .json()
        .await
        .context("decode query task response failed")?;

    if !body.ok {
        let error = telegram_provider_invalid_response(
            "query_task",
            body.error.as_deref().unwrap_or("application_rejected"),
        );
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    body.data
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.query_task_missing_data")))
}

pub(super) async fn submit_task_only(
    state: &BotState,
    user_id: i64,
    chat_id: i64,
    message_id: Option<String>,
    kind: TaskKind,
    mut payload: serde_json::Value,
) -> anyhow::Result<String> {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "telegram_bot_name".to_string(),
            json!(state.bot_name.clone()),
        );
    }
    let user_key = state
        .bound_identity_by_chat
        .lock()
        .ok()
        .and_then(|map| map.get(&chat_id).map(|identity| identity.user_key.clone()));
    let user_key_header = user_key.clone();
    let payload_compact = payload.to_string();
    let payload_fp = terminal_text_fingerprint_hex(&payload_compact);
    let payload_preview = terminal_text_preview_for_log(&payload_compact, 180);
    debug!(
        "phase=submit user_id={} chat_id={} kind={:?} payload_fp={} payload_len={} payload_preview={}",
        user_id,
        chat_id,
        kind,
        payload_fp,
        payload_compact.len(),
        payload_preview
    );
    let submit_req = SubmitTaskRequest {
        user_id: Some(user_id),
        chat_id: Some(chat_id),
        user_key,
        channel: Some(ChannelKind::Telegram),
        external_user_id: Some(user_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        ingress: Some({
            let mut ingress = claw_core::channel_ingress::ChannelIngressEnvelope::new(
                ChannelKind::Telegram,
                "telegram_bot",
            )
            .with_external_ids(user_id.to_string(), chat_id.to_string())
            .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::chat(
                chat_id.to_string(),
            ))
            .with_locale(state.language.clone());
            if let Some(message_id) = message_id.as_deref() {
                ingress = ingress.with_message_id(message_id);
            }
            ingress
        }),
        idempotency_key: message_id
            .as_deref()
            .map(|value| format!("telegram:{}:{value}", state.bot_name)),
        kind: kind.clone(),
        payload,
    };

    let submit_url = format!("{}/v1/tasks", state.clawd_base_url);
    debug!(
        "submit_task_only: url={} user_id={} chat_id={} kind={:?}",
        submit_url, user_id, chat_id, submit_req.kind
    );
    let submit_resp =
        maybe_with_user_key_header(state.client.post(&submit_url), user_key_header.as_deref())
            .json(&submit_req)
            .send()
            .await
            .context("submit task request failed")?;

    if !submit_resp.status().is_success() {
        let status = submit_resp.status();
        let body = submit_resp.text().await.unwrap_or_default();
        let error = telegram_provider_http_error("submit_task", status, &body);
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let submit_body: ApiResponse<SubmitTaskResponse> = submit_resp
        .json()
        .await
        .context("decode submit task response failed")?;

    if !submit_body.ok {
        let error = telegram_provider_invalid_response(
            "submit_task",
            submit_body
                .error
                .as_deref()
                .unwrap_or("application_rejected"),
        );
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let task_id = submit_body
        .data
        .ok_or_else(|| {
            anyhow!(
                "{}",
                state.i18n.t("telegram.error.submit_task_missing_task_id")
            )
        })?
        .task_id;

    debug!(
        "phase=submit_done user_id={} chat_id={} kind={:?} task_id={} payload_fp={}",
        user_id, chat_id, kind, task_id, payload_fp
    );
    Ok(task_id.to_string())
}

pub(super) async fn poll_task_result(
    state: &BotState,
    task_id: &str,
    user_key: Option<&str>,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<Vec<String>> {
    let poll_interval_ms = state.poll_interval_ms.max(1);
    let wait_seconds = wait_override_seconds
        .unwrap_or(state.task_wait_seconds)
        .max(1);
    let max_rounds = ((wait_seconds * 1000) / poll_interval_ms).max(1);

    for _ in 0..max_rounds {
        let task = query_task_status(state, task_id, user_key).await?;
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
            TaskStatus::Succeeded => {
                return Ok(task_success_messages(state, &task));
            }
            TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                return Err(anyhow!("{}", task_terminal_error_text(state, &task)));
            }
        }
    }

    Err(anyhow!("task_result_wait_timeout"))
}

pub(super) async fn cancel_tasks_for_chat(
    state: &BotState,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<i64> {
    let url = format!("{}/v1/tasks/cancel", state.clawd_base_url);
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
    });
    let resp = maybe_with_user_key_header(
        state.client.post(&url),
        bound_user_key_for_chat(state, chat_id).as_deref(),
    )
    .json(&payload)
    .send()
    .await
    .context("request cancel tasks failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let error = telegram_provider_http_error("cancel_task", status, &body);
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let body: ApiResponse<JsonValue> =
        resp.json().await.context("decode cancel response failed")?;

    if !body.ok {
        let error = telegram_provider_invalid_response(
            "cancel_task",
            body.error.as_deref().unwrap_or("application_rejected"),
        );
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let canceled = body
        .data
        .and_then(|v| v.get("canceled").and_then(|n| n.as_i64()))
        .unwrap_or(0);
    Ok(canceled)
}
pub(super) async fn fetch_status_text(state: &BotState, chat_id: i64) -> anyhow::Result<String> {
    let url = format!("{}/v1/health", state.clawd_base_url);
    let resp = maybe_with_user_key_header(
        state.client.get(&url),
        bound_user_key_for_chat(state, chat_id).as_deref(),
    )
    .send()
    .await
    .context("request health failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let error = telegram_provider_http_error("health_check", status, &body);
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let body: ApiResponse<HealthResponse> =
        resp.json().await.context("decode health response failed")?;

    if !body.ok {
        let error = telegram_provider_invalid_response(
            "health_check",
            body.error.as_deref().unwrap_or("application_rejected"),
        );
        return Err(anyhow!("{}", state.i18n.t(&error.message_key)));
    }

    let data = body
        .data
        .ok_or_else(|| anyhow!("{}", state.i18n.t("telegram.error.health_missing_data")))?;
    Ok(state.i18n.t_with(
        "telegram.msg.status_text",
        &[
            ("worker_state", &data.worker_state),
            ("queue_length", &data.queue_length.to_string()),
            ("running_length", &data.running_length.to_string()),
            (
                "running_oldest_age_seconds",
                &data.running_oldest_age_seconds.to_string(),
            ),
            (
                "task_timeout_seconds",
                &data.task_timeout_seconds.to_string(),
            ),
            ("uptime_seconds", &data.uptime_seconds.to_string()),
            ("version", &data.version),
        ],
    ))
}
