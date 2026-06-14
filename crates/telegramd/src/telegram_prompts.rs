use super::*;

pub(super) fn prompt_vendor_name_from_selected_vendor(selected_vendor: Option<&str>) -> String {
    selected_vendor
        .map(prompt_layers::normalize_prompt_vendor_name)
        .unwrap_or_else(|| "default".to_string())
}

pub(super) fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(super) fn load_prompt_template(
    workspace_root: &Path,
    prompt_vendor: &str,
    rel_path: &str,
    default_template: &str,
) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor,
        rel_path,
        default_template,
    )
    .0
}

pub(super) fn render_voice_chat_prompt(template: &str, transcript: &str) -> String {
    template.replace("__TRANSCRIPT__", transcript.trim())
}

pub(super) fn render_voice_mode_intent_prompt(template: &str, user_text: &str) -> String {
    template.replace("__USER_TEXT__", user_text.trim())
}

pub(super) fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif"
    )
}

pub(super) fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(super) struct TypingHeartbeatGuard {
    stop_tx: Option<oneshot::Sender<()>>,
}

impl TypingHeartbeatGuard {
    pub(super) fn start(bot: Bot, chat_id: ChatId) -> Self {
        // Telegram 的 typing 状态约 5 秒后过期，需在过期前重新发送以保持「正在输入」持续显示直到回复。
        const TYPING_REFRESH_INTERVAL_SECS: u64 = 4;
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(TYPING_REFRESH_INTERVAL_SECS)) => {}
                    _ = &mut stop_rx => break,
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
        }
    }

    fn stop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
    }
}

impl Drop for TypingHeartbeatGuard {
    fn drop(&mut self) {
        self.stop();
    }
}
