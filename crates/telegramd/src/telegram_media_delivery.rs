use super::*;
use claw_core::channel_media_limits::{
    preflight_local_media_file, LocalMediaPreflightFailure, MIB,
};
use claw_core::task_delivery_artifacts::is_managed_task_delivery_artifact_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TelegramMediaKind {
    Image,
    Video,
    File,
    Voice,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramUploadMethod {
    Photo,
    Video,
    Document,
    Voice,
    Audio,
}

fn telegram_upload_method(kind: TelegramMediaKind, size_bytes: u64) -> TelegramUploadMethod {
    match kind {
        TelegramMediaKind::Image
            if size_bytes <= claw_core::channel_media_limits::telegram_image_max_bytes() =>
        {
            TelegramUploadMethod::Photo
        }
        TelegramMediaKind::Image | TelegramMediaKind::File => TelegramUploadMethod::Document,
        TelegramMediaKind::Video => TelegramUploadMethod::Video,
        TelegramMediaKind::Voice => TelegramUploadMethod::Voice,
        TelegramMediaKind::Audio => TelegramUploadMethod::Audio,
    }
}

pub(super) fn telegram_media_filename(path: &Path) -> String {
    if let Some(filename) = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
    {
        filename.to_string()
    } else {
        path.to_string_lossy().to_string()
    }
}

fn telegram_media_failure_text(
    i18n: &TextCatalog,
    path: &Path,
    failure: LocalMediaPreflightFailure,
    actual_bytes: Option<u64>,
    max_bytes: u64,
    ui_fallback_available: bool,
) -> String {
    let reason = match failure {
        LocalMediaPreflightFailure::Unreadable => {
            i18n.t("telegram.msg.delivery_media_reason_unreadable")
        }
        LocalMediaPreflightFailure::NotRegularFile => {
            i18n.t("telegram.msg.delivery_media_reason_not_regular_file")
        }
        LocalMediaPreflightFailure::Empty => i18n.t("telegram.msg.delivery_media_reason_empty"),
        LocalMediaPreflightFailure::TooLarge => i18n.t_with(
            "telegram.msg.delivery_media_reason_too_large",
            &[
                (
                    "actual_mib",
                    &format!(
                        "{:.2}",
                        actual_bytes.unwrap_or_default() as f64 / MIB as f64
                    ),
                ),
                ("max_mib", &format!("{:.0}", max_bytes as f64 / MIB as f64)),
            ],
        ),
    };
    telegram_media_fallback_text(i18n, path, &reason, ui_fallback_available)
}

fn telegram_media_provider_failure_text(
    i18n: &TextCatalog,
    path: &Path,
    ui_fallback_available: bool,
) -> String {
    telegram_media_fallback_text(
        i18n,
        path,
        &i18n.t("telegram.msg.delivery_media_reason_provider_failed"),
        ui_fallback_available,
    )
}

fn telegram_media_fallback_text(
    i18n: &TextCatalog,
    path: &Path,
    reason: &str,
    ui_fallback_available: bool,
) -> String {
    let key = if ui_fallback_available {
        "telegram.msg.delivery_media_failed_ui_fallback"
    } else {
        "telegram.msg.delivery_media_failed_retry"
    };
    i18n.t_with(
        key,
        &[
            ("filename", &telegram_media_filename(path)),
            ("reason", reason),
        ],
    )
}

async fn send_telegram_media_fallback(
    bot: &Bot,
    chat_id: ChatId,
    path: &Path,
    text: String,
    error_code: &str,
    ui_fallback_available: bool,
) -> anyhow::Result<()> {
    warn!(
        error_code,
        chat_id = chat_id.0,
        filename = %telegram_media_filename(path),
        ui_fallback_available,
        "telegram media delivery fell back to a user notice"
    );
    send_telegram_text(bot, chat_id, &text)
        .await
        .context("send Telegram media fallback notice failed")?;
    Ok(())
}

pub(super) async fn deliver_telegram_media_path(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    path: &str,
    kind: TelegramMediaKind,
) -> anyhow::Result<()> {
    let path = Path::new(path);
    let ui_fallback_available = is_managed_task_delivery_artifact_path(&state.workspace_root, path);
    let size_bytes = match preflight_local_media_file(
        path,
        claw_core::channel_media_limits::telegram_file_max_bytes(),
    ) {
        Ok(size_bytes) => size_bytes,
        Err(error) => {
            let text = telegram_media_failure_text(
                state.i18n.as_ref(),
                path,
                error.failure,
                error.actual_bytes,
                error.max_bytes,
                ui_fallback_available,
            );
            return send_telegram_media_fallback(
                bot,
                chat_id,
                path,
                text,
                error.error_code(),
                ui_fallback_available,
            )
            .await;
        }
    };

    let method = telegram_upload_method(kind, size_bytes);
    let input = || InputFile::file(path.to_path_buf());
    let primary_result = match method {
        TelegramUploadMethod::Photo => bot.send_photo(chat_id, input()).await.map(|_| ()),
        TelegramUploadMethod::Video => bot.send_video(chat_id, input()).await.map(|_| ()),
        TelegramUploadMethod::Document => bot.send_document(chat_id, input()).await.map(|_| ()),
        TelegramUploadMethod::Voice => bot.send_voice(chat_id, input()).await.map(|_| ()),
        TelegramUploadMethod::Audio => bot.send_audio(chat_id, input()).await.map(|_| ()),
    };
    let primary_error = match primary_result {
        Ok(()) => return Ok(()),
        Err(error) => telegram_request_error("send_media", &error),
    };
    warn!(
        error_code = "telegram_media_native_upload_failed",
        provider_failure_class = primary_error.failure_class.as_str(),
        diagnostic_id = %primary_error.diagnostic_id,
        chat_id = chat_id.0,
        filename = %telegram_media_filename(path),
        upload_method = ?method,
        "Telegram native media upload failed"
    );

    if method != TelegramUploadMethod::Document {
        match bot.send_document(chat_id, input()).await {
            Ok(_) => return Ok(()),
            Err(error) => {
                let fallback_error = telegram_request_error("send_document", &error);
                warn!(
                    provider_failure_class = fallback_error.failure_class.as_str(),
                    diagnostic_id = %fallback_error.diagnostic_id,
                    "Telegram document fallback failed"
                );
            }
        }
    }

    let text =
        telegram_media_provider_failure_text(state.i18n.as_ref(), path, ui_fallback_available);
    send_telegram_media_fallback(
        bot,
        chat_id,
        path,
        text,
        "telegram_media_upload_failed",
        ui_fallback_available,
    )
    .await
}

pub(super) async fn deliver_missing_telegram_media_path(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    path: &str,
) -> anyhow::Result<()> {
    let path = Path::new(path);
    let text = telegram_media_failure_text(
        state.i18n.as_ref(),
        path,
        LocalMediaPreflightFailure::Unreadable,
        None,
        claw_core::channel_media_limits::telegram_file_max_bytes(),
        false,
    );
    send_telegram_media_fallback(bot, chat_id, path, text, "channel_media_unreadable", false).await
}

#[cfg(test)]
#[path = "telegram_media_delivery_tests.rs"]
mod tests;
