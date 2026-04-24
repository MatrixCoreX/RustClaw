use crate::{AppState, ClaimedTask};

pub(crate) fn text_contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

pub(crate) fn text_contains_ascii_alpha(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_alphabetic())
}

pub(crate) fn request_language_hint(user_text: &str) -> &'static str {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return "config_default";
    }
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => "zh-CN",
        (false, true) => "en",
        (true, true) => "mixed",
        (false, false) => "config_default",
    }
}

fn locale_hint_to_language_hint(locale_hint: &str) -> Option<&'static str> {
    let trimmed = locale_hint.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.to_ascii_lowercase();
    if normalized.starts_with("zh") {
        Some("zh-CN")
    } else if normalized.starts_with("en") {
        Some("en")
    } else {
        None
    }
}

pub(crate) fn preferred_response_language_hint(
    user_text: &str,
    session_locale_hint: Option<&str>,
) -> String {
    let request_hint = request_language_hint(user_text);
    if request_hint != "config_default" {
        return request_hint.to_string();
    }
    if let Some(locale_hint) = session_locale_hint.and_then(locale_hint_to_language_hint) {
        return locale_hint.to_string();
    }
    "config_default".to_string()
}

pub(crate) fn task_response_language_hint(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
) -> String {
    let session_snapshot = crate::conversation_state::load_active_session_snapshot(state, task);
    let session_locale_hint = session_snapshot
        .conversation_state
        .as_ref()
        .and_then(|conversation_state| conversation_state.locale_hint.as_deref());
    preferred_response_language_hint(user_text, session_locale_hint)
}

#[cfg(test)]
mod tests {
    use super::{preferred_response_language_hint, request_language_hint};

    #[test]
    fn request_language_hint_prefers_current_turn_text_shape() {
        assert_eq!(request_language_hint("写个两句短诗"), "zh-CN");
        assert_eq!(
            request_language_hint("do not run anything, just tell me a very short joke"),
            "en"
        );
        assert_eq!(request_language_hint("用 English 解释 README"), "mixed");
        assert_eq!(request_language_hint("12345"), "config_default");
    }

    #[test]
    fn preferred_response_language_hint_falls_back_to_session_locale_when_turn_is_ambiguous() {
        assert_eq!(
            preferred_response_language_hint("continue", Some("en-US")),
            "en"
        );
        assert_eq!(
            preferred_response_language_hint("继续", Some("en-US")),
            "zh-CN"
        );
        assert_eq!(
            preferred_response_language_hint("12345", Some("zh-CN")),
            "zh-CN"
        );
        assert_eq!(
            preferred_response_language_hint("12345", Some("fr-FR")),
            "config_default"
        );
    }
}
