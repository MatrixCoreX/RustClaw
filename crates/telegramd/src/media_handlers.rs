use super::*;

pub(super) async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    _state: BotState,
) -> anyhow::Result<()> {
    // Inline keyboards for crypto trade confirmation were removed; dismiss loading state for stray taps.
    if q.data.is_some() {
        let _ = bot.answer_callback_query(q.id).await;
    }
    Ok(())
}

pub(super) async fn handle_image_only_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let queue_len = match fetch_queue_length(state, msg.chat.id.0).await {
        Ok(v) => v,
        Err(_) => 0,
    };
    if queue_len >= state.queue_limit {
        bot.send_message(
            msg.chat.id,
            state.i18n.t_with(
                "telegram.msg.queue_full",
                &[
                    ("queued", &queue_len.to_string()),
                    ("limit", &state.queue_limit.to_string()),
                ],
            ),
        )
        .await
        .context("send queue full image message failed")?;
        return Ok(());
    }

    let ts = unix_ts();
    let normalized_ext = normalize_image_ext(ext);
    let rel_path = build_telegram_inbox_rel_path(
        &state.image_inbox_dir,
        &state.bot_name,
        msg.chat.id.0,
        user_id,
        ts,
        &normalized_ext,
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);

    download_telegram_file(state, bot, file_id, &abs_path).await?;

    let args = json!({
        "action": "describe",
        "images": [{"path": rel_path}],
        "detail_level": "normal"
    });
    let payload = json!({
        "skill_name": "image_vision",
        "args": args
    });

    match submit_task_only(state, user_id, msg.chat.id.0, TaskKind::RunSkill, payload).await {
        Ok(task_id) => {
            spawn_task_result_delivery(
                bot.clone(),
                state.clone(),
                msg.chat.id,
                user_id,
                task_id,
                None,
                state.i18n.t("telegram.msg.skill_exec_failed"),
            );
        }
        Err(err) => {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.skill_exec_failed_with_error",
                    &[("error", &err.to_string())],
                ),
            )
            .await
            .context("send image vision submit error failed")?;
        }
    }

    Ok(())
}

pub(super) async fn handle_image_only_save_only(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let ts = unix_ts();
    let normalized_ext = normalize_image_ext(ext);
    let rel_path = build_telegram_inbox_rel_path(
        &state.image_inbox_dir,
        &state.bot_name,
        msg.chat.id.0,
        user_id,
        ts,
        &normalized_ext,
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_telegram_file(state, bot, file_id, &abs_path).await?;
    if let Ok(mut m) = state.pending_image_by_chat.lock() {
        m.insert(msg.chat.id.0, rel_path.clone());
    }
    bot.send_message(
        msg.chat.id,
        state.i18n.t("telegram.msg.image_received_wait_prompt"),
    )
    .await
    .context("send image saved path message failed")?;
    Ok(())
}

pub(super) async fn handle_audio_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let ts = unix_ts();
    let normalized_ext = normalize_audio_ext(ext);
    let rel_path = build_telegram_inbox_rel_path(
        &state.audio_inbox_dir,
        &state.bot_name,
        msg.chat.id.0,
        user_id,
        ts,
        &normalized_ext,
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_telegram_file(state, bot, file_id, &abs_path).await?;
    if let Ok(meta) = tokio::fs::metadata(&abs_path).await {
        if meta.len() as usize > state.max_audio_input_bytes {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.audio_too_large",
                    &[
                        ("size", &meta.len().to_string()),
                        ("limit", &state.max_audio_input_bytes.to_string()),
                    ],
                ),
            )
            .await
            .context("send audio too large message failed")?;
            return Ok(());
        }
    }

    let _typing_guard = TypingHeartbeatGuard::start(bot.clone(), msg.chat.id);
    let transcribe_payload = json!({
        "skill_name": "audio_transcribe",
        "args": {
            "audio": { "path": rel_path }
        }
    });
    let transcribe_task_id = submit_task_only(
        state,
        user_id,
        msg.chat.id.0,
        TaskKind::RunSkill,
        transcribe_payload,
    )
    .await
    .context("submit audio_transcribe task failed")?;
    let transcript = poll_task_result(
        state,
        &transcribe_task_id,
        bound_user_key_for_chat(state, msg.chat.id.0).as_deref(),
        Some(120),
    )
    .await
    .context("poll audio_transcribe result failed")?;
    let transcript = transcript.join("\n").trim().to_string();
    let transcript = transcript.as_str();
    if transcript.is_empty() {
        bot.send_message(
            msg.chat.id,
            state.i18n.t("telegram.msg.audio_transcript_empty"),
        )
        .await
        .context("send empty transcript message failed")?;
        return Ok(());
    }

    info!(
        "{} transport_prompt_use flow=voice_chat prompt_name=voice_chat_prompt chat_id={} user_id={} prompt_source={}",
        transport_highlight_tag("transport_prompt"),
        msg.chat.id.0,
        user_id,
        VOICE_CHAT_PROMPT_LOGICAL_PATH
    );
    let ask_task_id = submit_task_only(
        state,
        user_id,
        msg.chat.id.0,
        TaskKind::Ask,
        json!({
            "text": render_voice_chat_prompt(&state.voice_chat_prompt_template, transcript),
            "source": "voice"
        }),
    )
    .await
    .context("submit ask task for transcript failed")?;
    let answers = poll_task_result(
        state,
        &ask_task_id,
        bound_user_key_for_chat(state, msg.chat.id.0).as_deref(),
        Some(state.task_wait_seconds.max(300)),
    )
    .await
    .context("poll ask result for transcript failed")?;
    let answer_joined = answers.join("\n\n");
    let mode = parse_voice_reply_mode(&effective_voice_reply_mode_for_chat(state, msg.chat.id.0));
    if matches!(mode, VoiceReplyMode::Text | VoiceReplyMode::Both) {
        for answer in &answers {
            send_text_or_image(bot, state, msg.chat.id, answer).await?;
        }
    }

    if matches!(mode, VoiceReplyMode::Voice | VoiceReplyMode::Both) {
        let tts_input = strip_delivery_tokens_for_tts(&answer_joined);
        if !tts_input.is_empty() {
            let tts_payload = json!({
                "skill_name": "audio_synthesize",
                "args": {
                    "text": tts_input,
                    "response_format": "opus"
                }
            });
            match submit_task_only(
                state,
                user_id,
                msg.chat.id.0,
                TaskKind::RunSkill,
                tts_payload,
            )
            .await
            {
                Ok(tts_task_id) => match poll_task_result(
                    state,
                    &tts_task_id,
                    bound_user_key_for_chat(state, msg.chat.id.0).as_deref(),
                    Some(90),
                )
                .await
                {
                    Ok(tts_answer) => {
                        for msg_text in tts_answer {
                            let _ = send_text_or_image(bot, state, msg.chat.id, &msg_text).await;
                        }
                    }
                    Err(err) => {
                        warn!("audio_synthesize poll failed: {err}");
                    }
                },
                Err(err) => {
                    warn!("submit audio_synthesize failed: {err}");
                }
            }
        } else if matches!(mode, VoiceReplyMode::Voice) {
            // Voice-only mode but no speakable text: fallback to original answer.
            for answer in &answers {
                send_text_or_image(bot, state, msg.chat.id, answer).await?;
            }
        }
    }
    Ok(())
}

pub(super) async fn handle_file_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let ts = unix_ts();
    let normalized_ext = normalize_file_ext(ext);
    let rel_path = build_telegram_inbox_rel_path(
        &state.file_inbox_dir,
        &state.bot_name,
        msg.chat.id.0,
        user_id,
        ts,
        &normalized_ext,
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_telegram_file(state, bot, file_id, &abs_path).await?;
    bot.send_message(
        msg.chat.id,
        state
            .i18n
            .t_with("telegram.msg.file_saved_path", &[("path", &rel_path)]),
    )
    .await
    .context("send file saved path message failed")?;
    Ok(())
}

pub(super) async fn handle_video_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
) -> anyhow::Result<()> {
    let ts = unix_ts();
    let normalized_ext = normalize_video_ext(ext);
    let rel_path = build_telegram_inbox_rel_path(
        &state.video_inbox_dir,
        &state.bot_name,
        msg.chat.id.0,
        user_id,
        ts,
        &normalized_ext,
    );
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_telegram_file(state, bot, file_id, &abs_path).await?;
    bot.send_message(
        msg.chat.id,
        state
            .i18n
            .t_with("telegram.msg.video_saved_path", &[("path", &rel_path)]),
    )
    .await
    .context("send video saved path message failed")?;
    Ok(())
}

pub(super) fn extract_image_attachment(msg: &Message) -> Option<(String, String)> {
    let MessageKind::Common(common) = &msg.kind else {
        return None;
    };
    match &common.media_kind {
        MediaKind::Photo(media) => media
            .photo
            .last()
            .map(|photo| (photo.file.id.to_string(), "jpg".to_string())),
        MediaKind::Document(media) => {
            let file_name_ext = media
                .document
                .file_name
                .as_deref()
                .and_then(extension_from_filename)
                .unwrap_or_default();
            let mime_is_image = media
                .document
                .mime_type
                .as_ref()
                .map(|m| m.type_().as_str() == "image")
                .unwrap_or(false);
            if mime_is_image || is_image_ext(&file_name_ext) {
                let ext = if file_name_ext.is_empty() {
                    "png".to_string()
                } else {
                    file_name_ext
                };
                Some((media.document.file.id.to_string(), ext))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn extract_audio_attachment(msg: &Message) -> Option<(String, String)> {
    let MessageKind::Common(common) = &msg.kind else {
        return None;
    };
    match &common.media_kind {
        MediaKind::Voice(media) => Some((media.voice.file.id.to_string(), "ogg".to_string())),
        MediaKind::Audio(media) => {
            let ext = media
                .audio
                .file_name
                .as_deref()
                .and_then(extension_from_filename)
                .unwrap_or_else(|| "mp3".to_string());
            Some((media.audio.file.id.to_string(), ext))
        }
        _ => None,
    }
}

pub(super) fn extract_video_attachment(msg: &Message) -> Option<(String, String)> {
    let MessageKind::Common(common) = &msg.kind else {
        return None;
    };
    match &common.media_kind {
        MediaKind::Video(media) => {
            let ext = media
                .video
                .file_name
                .as_deref()
                .and_then(extension_from_filename)
                .unwrap_or_else(|| "mp4".to_string());
            Some((media.video.file.id.to_string(), ext))
        }
        MediaKind::Document(media) => {
            let file_name_ext = media
                .document
                .file_name
                .as_deref()
                .and_then(extension_from_filename)
                .unwrap_or_default();
            let mime_is_video = media
                .document
                .mime_type
                .as_ref()
                .map(|m| m.type_().as_str() == "video")
                .unwrap_or(false);
            if mime_is_video
                || matches!(
                    file_name_ext.as_str(),
                    "mp4" | "mov" | "webm" | "mkv" | "m4v"
                )
            {
                let ext = if file_name_ext.is_empty() {
                    "mp4".to_string()
                } else {
                    file_name_ext
                };
                Some((media.document.file.id.to_string(), ext))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn extract_file_attachment(msg: &Message) -> Option<(String, String)> {
    let MessageKind::Common(common) = &msg.kind else {
        return None;
    };
    let MediaKind::Document(media) = &common.media_kind else {
        return None;
    };
    let file_name_ext = media
        .document
        .file_name
        .as_deref()
        .and_then(extension_from_filename)
        .unwrap_or_default();
    let mime = media.document.mime_type.as_ref();
    let mime_type = mime.map(|m| m.type_().as_str()).unwrap_or("");
    let mime_subtype = mime.map(|m| m.subtype().as_str()).unwrap_or("");
    let looks_like_audio = mime_type == "audio"
        || matches!(
            mime_subtype,
            "ogg" | "mpeg" | "mp3" | "wav" | "x-wav" | "aac" | "flac" | "opus"
        )
        || matches!(
            file_name_ext.as_str(),
            "ogg" | "mp3" | "wav" | "m4a" | "aac" | "flac" | "opus"
        );
    if mime_type == "image" || is_image_ext(&file_name_ext) || looks_like_audio {
        return None;
    }
    let ext = if file_name_ext.is_empty() {
        "bin".to_string()
    } else {
        file_name_ext
    };
    Some((media.document.file.id.to_string(), ext))
}

pub(super) async fn download_telegram_file(
    state: &BotState,
    bot: &Bot,
    file_id: String,
    local_path: &Path,
) -> anyhow::Result<()> {
    let file = bot
        .get_file(file_id)
        .await
        .context("telegram get_file failed")?;
    let file_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        state.bot_token, file.path
    );
    let bytes = state
        .client
        .get(file_url)
        .send()
        .await
        .context("download telegram file request failed")?
        .bytes()
        .await
        .context("read telegram file bytes failed")?;
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("create telegram media inbox dir failed")?;
    }
    tokio::fs::write(local_path, &bytes)
        .await
        .context("write downloaded file failed")?;
    Ok(())
}

pub(super) fn extension_from_filename(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_string_lossy().to_string();
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

pub(super) fn normalize_image_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if is_image_ext(&e) {
        e
    } else {
        "png".to_string()
    }
}

pub(super) fn normalize_audio_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if matches!(
        e.as_str(),
        "ogg" | "mp3" | "wav" | "m4a" | "aac" | "flac" | "opus"
    ) {
        e
    } else {
        "ogg".to_string()
    }
}

pub(super) fn normalize_file_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if e.is_empty() {
        "bin".to_string()
    } else {
        e
    }
}

pub(super) fn normalize_video_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if matches!(e.as_str(), "mp4" | "mov" | "webm" | "mkv" | "m4v") {
        e
    } else {
        "mp4".to_string()
    }
}
