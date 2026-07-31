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
                            deliver_voice_answers(&bot, &state, chat_id, user_id, &answers).await;
                        } else {
                            for answer in answers {
                                debug!(
                                    "phase=deliver_success_item task_id={} chat_id={} msg_fp={} msg_len={} msg_preview={}",
                                    task_id,
                                    chat_id.0,
                                    text_fingerprint_hex(&answer),
                                    answer.len(),
                                    text_preview_for_log(&answer, 160)
                                );
                                let _ = send_success_message_for_telegram(
                                    &bot, &state, chat_id, &answer,
                                )
                                .await;
                            }
                        }
                        break;
                    }
                    TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                        let detail = task_terminal_error_text(&state, &task);
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
                            let fail_msg = format!(
                                "{}",
                                state.i18n.t_with(
                                    "telegram.msg.resume_interrupted_hint",
                                    &[("prefix", &fail_prefix), ("detail", &detail)],
                                )
                            );
                            let _ = bot.send_message(chat_id, fail_msg).await;
                            break;
                        }
                        let _ = bot
                            .send_message(chat_id, format!("{fail_prefix}：{detail}"))
                            .await;
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
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    user_id: i64,
    answers: &[String],
) {
    let mode = parse_voice_reply_mode(&effective_voice_reply_mode_for_chat(state, chat_id.0));
    let mut media_delivered_for_voice_mode = false;
    if matches!(mode, VoiceReplyMode::Text | VoiceReplyMode::Both) {
        for answer in answers {
            let _ = send_success_message_for_telegram(bot, state, chat_id, answer).await;
        }
    } else if matches!(mode, VoiceReplyMode::Voice) {
        for answer in answers {
            let delivery_tokens = delivery_tokens_only(answer);
            if delivery_tokens.is_empty() {
                continue;
            }
            let _ = send_success_message_for_telegram(bot, state, chat_id, &delivery_tokens).await;
            media_delivered_for_voice_mode = true;
        }
    }
    if !matches!(mode, VoiceReplyMode::Voice | VoiceReplyMode::Both) {
        return;
    }

    let tts_input = strip_delivery_tokens_for_tts(&answers.join("\n\n"));
    if tts_input.is_empty() {
        if matches!(mode, VoiceReplyMode::Voice) && !media_delivered_for_voice_mode {
            for answer in answers {
                let _ = send_success_message_for_telegram(bot, state, chat_id, answer).await;
            }
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
            let _ = send_telegram_text(bot, chat_id, &tts_input).await;
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
        Ok(messages) => {
            for message in messages {
                let _ = send_success_message_for_telegram(bot, state, chat_id, &message).await;
            }
        }
        Err(err) => {
            warn!("telegram voice reply synthesis failed: {err}");
            if matches!(mode, VoiceReplyMode::Voice) {
                let _ = send_telegram_text(bot, chat_id, &tts_input).await;
            }
        }
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

pub(super) async fn send_success_message_for_telegram(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    answer: &str,
) -> anyhow::Result<()> {
    if let Some(blocks) = split_subtask_success_messages(answer) {
        for (header, body) in blocks {
            if body.is_empty() {
                send_telegram_text(bot, chat_id, &header)
                    .await
                    .context("send subtask header failed")?;
                continue;
            }
            if should_send_subtask_body_as_file(&header, &body) {
                let file_path = write_subtask_body_to_temp_file(&header, &body)?;
                let answer_with_file = format!("{header}\nFILE:{file_path}");
                send_text_or_image(bot, state, chat_id, &answer_with_file).await?;
                continue;
            }
            let html = format!(
                "{}\n<pre><code>{}</code></pre>",
                escape_telegram_html(&header),
                escape_telegram_html(&body)
            );
            bot.send_message(chat_id, html)
                .parse_mode(ParseMode::Html)
                .await
                .context("send subtask code block failed")?;
        }
        return Ok(());
    }
    send_text_or_image(bot, state, chat_id, answer).await
}

pub(super) fn split_subtask_success_messages(text: &str) -> Option<Vec<(String, String)>> {
    let trimmed = text.trim();
    if !trimmed.starts_with("subtask#") {
        return None;
    }

    let mut raw_blocks = Vec::new();
    let mut current = String::new();

    for line in trimmed.lines() {
        let line = line.trim_end();
        if line.starts_with("subtask#") {
            if !current.trim().is_empty() {
                raw_blocks.push(current.trim().to_string());
                current.clear();
            }
            current.push_str(line);
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }

    if !current.trim().is_empty() {
        raw_blocks.push(current.trim().to_string());
    }

    let blocks = raw_blocks
        .into_iter()
        .map(|block| split_single_subtask_block(&block))
        .collect::<Vec<_>>();
    Some(blocks)
}

pub(super) fn split_single_subtask_block(block: &str) -> (String, String) {
    let trimmed = block.trim();
    let (first_line, rest) = match trimmed.split_once('\n') {
        Some((head, tail)) => (head.trim(), tail.trim()),
        None => (trimmed, ""),
    };

    if let Some((header, inline_body)) = first_line.split_once(" | ") {
        let mut body = inline_body.trim().to_string();
        if !rest.is_empty() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(rest);
        }
        return (header.trim().to_string(), body);
    }

    (first_line.to_string(), rest.to_string())
}

pub(super) fn should_send_subtask_body_as_file(header: &str, body: &str) -> bool {
    let html_len = escape_telegram_html(header).len() + escape_telegram_html(body).len() + 32;
    html_len > 3000 || body.lines().count() > 120
}

pub(super) fn write_subtask_body_to_temp_file(header: &str, body: &str) -> anyhow::Result<String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sanitized = sanitize_filename_fragment(header);
    let path = std::env::temp_dir().join(format!("agent-{sanitized}-{millis}.txt"));
    fs::write(&path, body)
        .with_context(|| format!("write subtask temp file failed: {}", path.display()))?;
    Ok(path.to_string_lossy().to_string())
}

pub(super) fn sanitize_filename_fragment(text: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in text.chars() {
        let keep = ch.is_ascii_alphanumeric();
        if keep {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "subtask".to_string()
    } else {
        trimmed
    }
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
        let mut out = dedupe_preserve_order(out);
        if !out.is_empty() {
            let has_explicit_delivery = out.iter().any(|msg| has_delivery_prefix(msg));
            if has_explicit_delivery {
                out.retain(|msg| !is_written_file_confirmation_line(msg));
            }
            debug!(
                "phase=success_source task_id={} source=messages offset={} messages_len={} explicit_delivery={}",
                task_id,
                offset,
                out.len(),
                has_explicit_delivery
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
        text_fingerprint_hex(&text),
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
    dedupe_preserve_order(out)
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
    payload: serde_json::Value,
) -> anyhow::Result<String> {
    let user_key = state
        .bound_identity_by_chat
        .lock()
        .ok()
        .and_then(|map| map.get(&chat_id).map(|identity| identity.user_key.clone()));
    let user_key_header = user_key.clone();
    let payload_compact = payload.to_string();
    let payload_fp = text_fingerprint_hex(&payload_compact);
    let payload_preview = text_preview_for_log(&payload_compact, 180);
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
            if let Some(message_id) = message_id {
                ingress = ingress.with_message_id(message_id);
            }
            ingress
        }),
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
