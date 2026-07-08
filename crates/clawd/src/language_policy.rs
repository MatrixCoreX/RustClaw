use std::borrow::Cow;

use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone, Copy, Default)]
struct TextLanguageCounts {
    cjk: usize,
    kana: usize,
    hangul: usize,
    ascii_alpha: usize,
    latin_extended: usize,
    cyrillic: usize,
    arabic: usize,
    devanagari: usize,
    greek: usize,
    hebrew: usize,
    thai: usize,
    other_alpha: usize,
}

impl TextLanguageCounts {
    fn non_cjk_alpha(self) -> usize {
        self.kana
            + self.hangul
            + self.ascii_alpha
            + self.latin_extended
            + self.cyrillic
            + self.arabic
            + self.devanagari
            + self.greek
            + self.hebrew
            + self.thai
            + self.other_alpha
    }

    fn non_cjk_non_ascii_alpha(self) -> usize {
        self.kana
            + self.hangul
            + self.latin_extended
            + self.cyrillic
            + self.arabic
            + self.devanagari
            + self.greek
            + self.hebrew
            + self.thai
            + self.other_alpha
    }
}

fn char_in_ranges(ch: char, ranges: &[(u32, u32)]) -> bool {
    let codepoint = ch as u32;
    ranges
        .iter()
        .any(|(start, end)| (*start..=*end).contains(&codepoint))
}

fn text_language_counts(text: &str) -> TextLanguageCounts {
    const CJK_RANGES: &[(u32, u32)] = &[(0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF)];
    const KANA_RANGES: &[(u32, u32)] = &[
        (0x3040, 0x309F),
        (0x30A0, 0x30FF),
        (0x31F0, 0x31FF),
        (0xFF66, 0xFF9D),
    ];
    const HANGUL_RANGES: &[(u32, u32)] = &[
        (0x1100, 0x11FF),
        (0x3130, 0x318F),
        (0xA960, 0xA97F),
        (0xAC00, 0xD7AF),
        (0xD7B0, 0xD7FF),
    ];
    const LATIN_EXTENDED_RANGES: &[(u32, u32)] =
        &[(0x00C0, 0x024F), (0x1E00, 0x1EFF), (0xA720, 0xA7FF)];
    const CYRILLIC_RANGES: &[(u32, u32)] = &[(0x0400, 0x052F), (0x2DE0, 0x2DFF), (0xA640, 0xA69F)];
    const ARABIC_RANGES: &[(u32, u32)] = &[
        (0x0600, 0x06FF),
        (0x0750, 0x077F),
        (0x08A0, 0x08FF),
        (0xFB50, 0xFDFF),
        (0xFE70, 0xFEFF),
    ];
    const DEVANAGARI_RANGES: &[(u32, u32)] = &[(0x0900, 0x097F), (0xA8E0, 0xA8FF)];
    const GREEK_RANGES: &[(u32, u32)] = &[(0x0370, 0x03FF), (0x1F00, 0x1FFF)];
    const HEBREW_RANGES: &[(u32, u32)] = &[(0x0590, 0x05FF)];
    const THAI_RANGES: &[(u32, u32)] = &[(0x0E00, 0x0E7F)];

    let mut counts = TextLanguageCounts::default();
    let mut cjk = 0usize;
    for ch in text.chars() {
        if char_in_ranges(ch, CJK_RANGES) {
            counts.cjk += 1;
            cjk += 1;
        } else if char_in_ranges(ch, KANA_RANGES) {
            counts.kana += 1;
        } else if char_in_ranges(ch, HANGUL_RANGES) {
            counts.hangul += 1;
        } else if char_in_ranges(ch, LATIN_EXTENDED_RANGES) && ch.is_alphabetic() {
            counts.latin_extended += 1;
        } else if char_in_ranges(ch, CYRILLIC_RANGES) {
            counts.cyrillic += 1;
        } else if char_in_ranges(ch, ARABIC_RANGES) {
            counts.arabic += 1;
        } else if char_in_ranges(ch, DEVANAGARI_RANGES) {
            counts.devanagari += 1;
        } else if char_in_ranges(ch, GREEK_RANGES) {
            counts.greek += 1;
        } else if char_in_ranges(ch, HEBREW_RANGES) {
            counts.hebrew += 1;
        } else if char_in_ranges(ch, THAI_RANGES) {
            counts.thai += 1;
        } else if ch.is_ascii_alphabetic() {
            counts.ascii_alpha += 1;
        } else if ch.is_alphabetic() {
            counts.other_alpha += 1;
        }
    }
    debug_assert_eq!(cjk, counts.cjk);
    counts
}

fn trim_language_neutral_token_edges(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        ch.is_ascii_punctuation() && !matches!(ch, '/' | '\\' | '.' | '_' | '-' | '~' | ':' | '@')
    })
}

fn looks_like_language_neutral_artifact_token(token: &str) -> bool {
    let token = trim_language_neutral_token_edges(token);
    if token.is_empty() {
        return false;
    }
    let counts = text_language_counts(token);
    if counts.cjk > 0 || counts.kana > 0 || counts.hangul > 0 || counts.ascii_alpha == 0 {
        return false;
    }
    if token.contains("://")
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('/')
        || token.starts_with("~/")
        || token.contains('/')
        || token.contains('\\')
    {
        return true;
    }
    let bytes = token.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
    {
        return true;
    }
    if token.contains('_') || token.contains('-') {
        return true;
    }
    if let Some((head, ext)) = token.rsplit_once('.') {
        let ext = ext.trim();
        if !head.is_empty()
            && !ext.is_empty()
            && ext.len() <= 12
            && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
        {
            return true;
        }
    }
    let alpha_count = token.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    alpha_count >= 2
        && token
            .chars()
            .filter(|ch| ch.is_ascii_alphabetic())
            .all(|ch| ch.is_ascii_uppercase())
}

fn looks_like_language_neutral_scalar_token(token: &str) -> bool {
    let token = trim_language_neutral_token_edges(token);
    if token.is_empty() {
        return false;
    }
    let mut saw_digit = false;
    for ch in token.chars() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if !matches!(ch, '.' | ',' | ':' | '+' | '-' | '%' | '=') {
            return false;
        }
    }
    saw_digit
}

pub(crate) fn text_is_language_neutral_artifact_only(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let mut saw_token = false;
    let mut saw_artifact = false;
    for token in trimmed.split_whitespace() {
        if token.trim().is_empty() {
            continue;
        }
        saw_token = true;
        if looks_like_language_neutral_artifact_token(token) {
            saw_artifact = true;
            continue;
        }
        if !looks_like_language_neutral_scalar_token(token) {
            return false;
        }
    }
    saw_token && saw_artifact
}

fn text_language_counts_without_neutral_artifacts(text: &str) -> TextLanguageCounts {
    let mut language_text = String::with_capacity(text.len());
    for token in text.split_whitespace() {
        if looks_like_language_neutral_artifact_token(token) {
            language_text.push(' ');
        } else {
            language_text.push_str(token);
            language_text.push(' ');
        }
    }
    text_language_counts(&language_text)
}

pub(crate) fn text_language_conflicts_with_hint(text: &str, language_hint: &str) -> bool {
    if text_is_language_neutral_artifact_only(text) {
        return false;
    }
    let counts = text_language_counts(text);
    let hint = language_hint.trim().to_ascii_lowercase();
    if hint.starts_with("en") {
        return counts.cjk > 0 && counts.cjk.saturating_mul(2) > counts.ascii_alpha;
    }
    if hint.starts_with("zh") {
        return counts.cjk == 0 && counts.ascii_alpha > 16;
    }
    if hint.starts_with("ko") {
        return counts.hangul == 0 && counts.ascii_alpha > 16;
    }
    if hint.starts_with("ja") {
        return counts.kana == 0 && counts.cjk == 0 && counts.ascii_alpha > 16;
    }
    false
}

pub(crate) fn request_language_hint(user_text: &str) -> &'static str {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return "config_default";
    }
    if text_is_language_neutral_artifact_only(trimmed) {
        return "config_default";
    }
    let counts = text_language_counts(trimmed);
    if counts.kana > 0 {
        return "ja";
    }
    if counts.hangul > 0 {
        return "ko";
    }
    if counts.cjk > 0 {
        let semantic_counts = text_language_counts_without_neutral_artifacts(trimmed);
        return if semantic_counts.cjk > 0 && semantic_counts.cjk >= semantic_counts.non_cjk_alpha()
        {
            "zh-CN"
        } else {
            "mixed"
        };
    }
    let non_ascii_alpha = counts.non_cjk_non_ascii_alpha();
    if counts.ascii_alpha > 0 && non_ascii_alpha == 0 {
        return "en";
    }
    if counts.latin_extended > 0 {
        return if counts.ascii_alpha > 0 || non_ascii_alpha == counts.latin_extended {
            "und-Latn"
        } else {
            "mixed"
        };
    }
    if counts.cyrillic > 0 && non_ascii_alpha == counts.cyrillic {
        return "und-Cyrl";
    }
    if counts.arabic > 0 && non_ascii_alpha == counts.arabic {
        return "und-Arab";
    }
    if counts.devanagari > 0 && non_ascii_alpha == counts.devanagari {
        return "und-Deva";
    }
    if counts.greek > 0 && non_ascii_alpha == counts.greek {
        return "und-Grek";
    }
    if counts.hebrew > 0 && non_ascii_alpha == counts.hebrew {
        return "he";
    }
    if counts.thai > 0 && non_ascii_alpha == counts.thai {
        return "th";
    }
    if non_ascii_alpha > 0 {
        return "mixed";
    }
    "config_default"
}

pub(crate) fn first_clear_request_language_hint<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    for candidate in candidates {
        let hint = request_language_hint(candidate.trim());
        if hint != "config_default" {
            return Some(hint.to_string());
        }
    }
    None
}

pub(crate) fn mixed_language_prefers_cjk_response(user_text: &str) -> bool {
    let counts = text_language_counts_without_neutral_artifacts(user_text.trim());
    counts.cjk > 0 && counts.cjk.saturating_mul(4) >= counts.non_cjk_alpha()
}

fn canonical_locale_hint(locale_hint: &str) -> Option<String> {
    let trimmed = locale_hint.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.replace('_', "-");
    if normalized.len() > 32
        || normalized
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-'))
    {
        return None;
    }
    let mut parts = normalized.split('-');
    let primary = parts.next()?.to_ascii_lowercase();
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    if primary == "zh" {
        return Some("zh-CN".to_string());
    }
    if primary == "en" {
        return Some("en".to_string());
    }
    let rest = parts
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part.len() == 2 && part.chars().all(|ch| ch.is_ascii_alphabetic()) {
                part.to_ascii_uppercase()
            } else if part.len() == 4 && part.chars().all(|ch| ch.is_ascii_alphabetic()) {
                let mut chars = part.chars();
                let first = chars.next().unwrap_or('x').to_ascii_uppercase();
                format!("{}{}", first, chars.as_str().to_ascii_lowercase())
            } else {
                part.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>();
    if rest.is_empty() {
        Some(primary)
    } else {
        Some(format!("{}-{}", primary, rest.join("-")))
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
    if let Some(locale_hint) = session_locale_hint.and_then(canonical_locale_hint) {
        return locale_hint;
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
    let mut task_merge_markers = 0usize;
    for line in text.lines().map(str::trim_start) {
        if matches!(line.trim(), "[AUTO_LOCATOR]") || line.starts_with("### RUNTIME_CONTEXT") {
            return true;
        }
        if line.starts_with("Current task:")
            || line.starts_with("Continuity rules:")
            || line.starts_with("Structured task updates:")
            || line.starts_with("New user instruction:")
        {
            task_merge_markers += 1;
        }
    }
    task_merge_markers >= 2
}

fn looks_like_locator_only_clarify_reply(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.chars().count() > 160 {
        return false;
    }
    if text_is_language_neutral_artifact_only(trimmed) {
        return true;
    }
    if trimmed.split_whitespace().count() != 1 {
        return false;
    }
    let token = trim_language_neutral_token_edges(trimmed);
    token.chars().count() >= 2
        && token.chars().any(|ch| ch.is_ascii_alphanumeric())
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '@'))
}

#[cfg(test)]
fn task_language_source_text<'a>(task: &'a ClaimedTask, user_text: &'a str) -> Cow<'a, str> {
    task_language_source_text_with_active_clarify(task, user_text, None)
}

fn task_language_source_text_with_active_clarify<'a>(
    task: &'a ClaimedTask,
    user_text: &'a str,
    active_clarify_source: Option<&'a str>,
) -> Cow<'a, str> {
    let payload_text = task_original_user_text(task);
    if let Some(source) = active_clarify_source
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let source_hint = request_language_hint(source);
        let locator_only_reply = looks_like_locator_only_clarify_reply(user_text)
            || payload_text
                .as_deref()
                .is_some_and(looks_like_locator_only_clarify_reply);
        if source_hint != "config_default" && locator_only_reply {
            return Cow::Borrowed(source);
        }
    }
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
    let active_clarify_source = session_snapshot
        .active_clarify_state
        .as_ref()
        .map(|clarify| clarify.source_request.as_str());
    let language_source =
        task_language_source_text_with_active_clarify(task, user_text, active_clarify_source);
    preferred_response_language_hint(&language_source, session_locale_hint)
}

#[cfg(test)]
#[path = "language_policy_tests.rs"]
mod tests;
