use std::borrow::Cow;

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

fn text_language_counts(text: &str) -> (usize, usize) {
    let mut cjk = 0usize;
    let mut ascii_alpha = 0usize;
    for ch in text.chars() {
        if matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        ) {
            cjk += 1;
        } else if ch.is_ascii_alphabetic() {
            ascii_alpha += 1;
        }
    }
    (cjk, ascii_alpha)
}

pub(crate) fn text_language_conflicts_with_hint(text: &str, language_hint: &str) -> bool {
    let (cjk, ascii_alpha) = text_language_counts(text);
    let hint = language_hint.trim().to_ascii_lowercase();
    if hint.starts_with("en") {
        return cjk > 0 && cjk.saturating_mul(2) > ascii_alpha;
    }
    if hint.starts_with("zh") {
        return cjk == 0 && ascii_alpha > 16;
    }
    false
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

fn task_payload_text_for_language(task: &ClaimedTask) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&task.payload_json)
        .ok()?
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn looks_like_placeholder_payload(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "placeholder" | "<placeholder>" | "(placeholder)"
    )
}

pub(crate) fn task_original_user_text(task: &ClaimedTask) -> Option<String> {
    task_payload_text_for_language(task).filter(|text| !looks_like_placeholder_payload(text))
}

fn looks_like_runtime_scaffold(text: &str) -> bool {
    [
        "[AUTO_LOCATOR]",
        "### RUNTIME_CONTEXT",
        "Current task:",
        "Continuity rules:",
        "Structured task updates:",
        "New user instruction:",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn task_language_source_text<'a>(task: &'a ClaimedTask, user_text: &'a str) -> Cow<'a, str> {
    if let Some(payload_text) = task_original_user_text(task) {
        let payload_hint = request_language_hint(&payload_text);
        if payload_hint != "config_default" && payload_text.trim() != user_text.trim() {
            return Cow::Owned(payload_text);
        }
    }
    let user_text_hint = request_language_hint(user_text);
    if user_text_hint != "config_default" && !looks_like_runtime_scaffold(user_text) {
        return Cow::Borrowed(user_text);
    }
    task_payload_text_for_language(task)
        .map(Cow::Owned)
        .unwrap_or_else(|| Cow::Borrowed(user_text))
}

pub(crate) fn task_user_request_for_prompt(task: &ClaimedTask, user_text: &str) -> String {
    let resolved = user_text.trim();
    let Some(original) = task_original_user_text(task) else {
        return resolved.to_string();
    };
    let original = original.trim();
    if original.is_empty() || original == resolved {
        return resolved.to_string();
    }
    format!(
        "Original user request:\n{original}\n\nResolved semantic request:\n{resolved}\n\nUse the resolved semantic request for planning/execution, but preserve the original user's language, final-output format, and wording constraints."
    )
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
    let language_source = task_language_source_text(task, user_text);
    preferred_response_language_hint(&language_source, session_locale_hint)
}

#[cfg(test)]
mod tests {
    use super::{
        preferred_response_language_hint, request_language_hint, task_language_source_text,
        task_user_request_for_prompt, text_language_conflicts_with_hint,
    };

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
    fn generated_text_language_conflict_allows_embedded_names() {
        assert!(text_language_conflicts_with_hint(
            "请具体说明您想继续什么任务或操作？",
            "en"
        ));
        assert!(!text_language_conflicts_with_hint(
            "Please confirm the target file 新加卷 before I continue.",
            "en"
        ));
        assert!(text_language_conflicts_with_hint(
            "I couldn't determine the requested action.",
            "zh-CN"
        ));
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

    #[test]
    fn task_language_source_prefers_original_payload_text_over_runtime_scaffold() {
        let task = crate::ClaimedTask {
            task_id: "task-language-source".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({"text":"写一段总结"}).to_string(),
        };
        let scaffolded =
            "Write a summary\n\n[AUTO_LOCATOR]\nResolved present workspace scope to: /tmp/project";
        assert_eq!(
            task_language_source_text(&task, scaffolded).as_ref(),
            "写一段总结"
        );
    }

    #[test]
    fn task_language_source_prefers_explicit_current_text_over_placeholder_payload() {
        let task = crate::ClaimedTask {
            task_id: "task-language-current-text".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({"text":"placeholder"}).to_string(),
        };
        assert_eq!(
            task_language_source_text(&task, "明天提醒我检查部署").as_ref(),
            "明天提醒我检查部署"
        );
    }

    #[test]
    fn task_language_source_prefers_original_payload_over_resolved_semantic_rewrite() {
        let task = crate::ClaimedTask {
            task_id: "task-language-original".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({
                "text":"读取 UI/package.json 里的 name，最后只用一行输出：一样或不一样"
            })
            .to_string(),
        };
        let resolved = "Read the name field from UI/package.json and answer same or different.";
        assert_eq!(
            task_language_source_text(&task, resolved).as_ref(),
            "读取 UI/package.json 里的 name，最后只用一行输出：一样或不一样"
        );
    }

    #[test]
    fn task_user_request_for_prompt_keeps_original_language_and_resolved_semantics() {
        let task = crate::ClaimedTask {
            task_id: "task-request-for-prompt".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({
                "text":"读取 UI/package.json 里的 name，最后只用一行输出：一样或不一样"
            })
            .to_string(),
        };
        let rendered = task_user_request_for_prompt(
            &task,
            "Read the name field from UI/package.json and answer same or different.",
        );
        assert!(rendered.contains("Original user request:"));
        assert!(rendered.contains("一样或不一样"));
        assert!(rendered.contains("Resolved semantic request:"));
        assert!(rendered.contains("Read the name field"));
        assert!(rendered.contains("preserve the original user's language"));
    }
}
