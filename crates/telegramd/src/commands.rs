use super::*;

pub(super) fn sanitize_message_text_for_log(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("/key ") {
        "/key <redacted>".to_string()
    } else {
        text.to_string()
    }
}

pub(super) fn clear_pending_resume_for_chat(state: &BotState, chat_id: i64) {
    if let Ok(mut guard) = state.pending_resume_by_chat.lock() {
        guard.remove(&chat_id);
    }
}
