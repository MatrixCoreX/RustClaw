use super::*;

pub(super) async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    _state: BotState,
) -> anyhow::Result<()> {
    // The transport owns no callback actions; dismiss loading state for stale buttons.
    if q.data.is_some() {
        let _ = bot.answer_callback_query(q.id).await;
    }
    Ok(())
}

pub(super) async fn handle_image_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
    prompt: &str,
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

    submit_attachment_ask(
        bot,
        msg,
        state,
        user_id,
        prompt,
        "image",
        &normalized_ext,
        rel_path,
        false,
    )
    .await
}

pub(super) async fn handle_audio_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
    prompt: &str,
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

    submit_attachment_ask(
        bot,
        msg,
        state,
        user_id,
        prompt,
        "audio",
        &normalized_ext,
        rel_path,
        true,
    )
    .await
}

pub(super) async fn handle_file_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
    prompt: &str,
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
    submit_attachment_ask(
        bot,
        msg,
        state,
        user_id,
        prompt,
        "file",
        &normalized_ext,
        rel_path,
        false,
    )
    .await
}

pub(super) async fn handle_video_message(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    file_id: String,
    ext: &str,
    prompt: &str,
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
    submit_attachment_ask(
        bot,
        msg,
        state,
        user_id,
        prompt,
        "video",
        &normalized_ext,
        rel_path,
        false,
    )
    .await
}

async fn submit_attachment_ask(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    user_id: i64,
    prompt: &str,
    kind: &str,
    ext: &str,
    rel_path: String,
    voice_reply: bool,
) -> anyhow::Result<()> {
    let size = tokio::fs::metadata(&rel_path)
        .await
        .ok()
        .map(|meta| meta.len());
    let payload = json!({
        "text": prompt.trim(),
        "source": "telegram",
        "attachments": [{
            "kind": kind,
            "path": rel_path,
            "mime_type": attachment_mime_type(kind, ext),
            "size": size,
        }]
    });
    match submit_task_only(state, user_id, msg.chat.id.0, TaskKind::Ask, payload).await {
        Ok(task_id) if voice_reply => spawn_voice_task_result_delivery(
            bot.clone(),
            state.clone(),
            msg.chat.id,
            user_id,
            task_id,
            state.i18n.t("telegram.msg.process_failed"),
        ),
        Ok(task_id) => spawn_task_result_delivery(
            bot.clone(),
            state.clone(),
            msg.chat.id,
            user_id,
            task_id,
            None,
            state.i18n.t("telegram.msg.process_failed"),
        ),
        Err(err) => {
            bot.send_message(
                msg.chat.id,
                state.i18n.t_with(
                    "telegram.msg.process_failed_with_error",
                    &[("error", &err.to_string())],
                ),
            )
            .await
            .context("send attachment task submit error failed")?;
        }
    }
    Ok(())
}

fn attachment_mime_type(kind: &str, ext: &str) -> String {
    let normalized = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    match (kind, normalized.as_str()) {
        ("image", "jpg" | "jpeg") => "image/jpeg".to_string(),
        ("image", "png") => "image/png".to_string(),
        ("image", "webp") => "image/webp".to_string(),
        ("image", "gif") => "image/gif".to_string(),
        ("audio", "ogg" | "opus") => "audio/ogg".to_string(),
        ("audio", "mp3") => "audio/mpeg".to_string(),
        ("audio", "wav") => "audio/wav".to_string(),
        ("audio", "m4a") => "audio/mp4".to_string(),
        ("video", "webm") => "video/webm".to_string(),
        ("video", "mov") => "video/quicktime".to_string(),
        ("video", "mp4") => "video/mp4".to_string(),
        ("file", "pdf") => "application/pdf".to_string(),
        ("file", "json") => "application/json".to_string(),
        ("file", "txt" | "md" | "csv") => "text/plain".to_string(),
        ("image", _) => format!("image/{normalized}"),
        ("audio", _) => format!("audio/{normalized}"),
        ("video", _) => format!("video/{normalized}"),
        _ => "application/octet-stream".to_string(),
    }
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

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif"
    )
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
