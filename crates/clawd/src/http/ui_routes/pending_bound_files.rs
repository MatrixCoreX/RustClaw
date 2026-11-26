use crate::AppState;
use claw_core::channel_workspace::{stage_inbox_files, telegram_inbox_path};
use claw_core::types::{AuthIdentity, ChannelKind, SubmitTaskRequest};
use std::path::Path;

/// A pending record is submitted before authentication. Only the actual
/// transport inbox for the newly bound user, chat and provider message may
/// supply local bytes after binding.
pub(super) fn restore_bound_files(
    state: &AppState,
    channel: &str,
    user: Option<&str>,
    chat: Option<&str>,
    actor: &AuthIdentity,
    request: &SubmitTaskRequest,
) -> Result<(), &'static str> {
    let Some(ingress) = request
        .ingress
        .as_ref()
        .filter(|i| !i.attachments.is_empty())
    else {
        return Ok(());
    };
    let unverified = "pending_request_attachment_unverified";
    if channel != "telegram"
        || ingress.channel != ChannelKind::Telegram
        || ingress.external_user_id.as_deref() != user
        || ingress.external_chat_id.as_deref() != chat
    {
        return Err(unverified);
    }
    let user: i64 = user.and_then(|s| s.parse().ok()).ok_or(unverified)?;
    let chat: i64 = chat.and_then(|s| s.parse().ok()).ok_or(unverified)?;
    let account = ingress.account_id.as_deref().ok_or(unverified)?;
    let message = ingress.message_id.as_deref().ok_or(unverified)?;
    let config_path = crate::runtime::active_runtime_config_path(state);
    let config = claw_core::config::AppConfig::load(&config_path.to_string_lossy())
        .map_err(|_| unverified)?;
    for item in &ingress.attachments {
        let dir = match item.kind.as_str() {
            "image" => &config.telegram.image_inbox_dir,
            "video" => &config.telegram.video_inbox_dir,
            "audio" => &config.telegram.audio_inbox_dir,
            "file" => &config.telegram.file_inbox_dir,
            _ => return Err(unverified),
        };
        let ext = Path::new(&item.path)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or(unverified)?;
        if !ext.bytes().all(|c| c.is_ascii_alphanumeric())
            || item.path != telegram_inbox_path(dir, account, chat, user, message, ext)
        {
            return Err(unverified);
        }
    }
    if crate::repo::pending_channel_requests::request_attachment_paths(request)
        .any(|p| !ingress.attachments.iter().any(|i| i.path == p))
    {
        return Err(unverified);
    }
    if !actor.role.eq_ignore_ascii_case("admin") {
        let own = state.identity_files_root(actor).map_err(|_| unverified)?;
        stage_inbox_files(&state.skill_rt.workspace_root, &own, &ingress.attachments)
            .map_err(|_| unverified)?;
    }
    Ok(())
}
