use std::sync::OnceLock;

use regex::Regex;

use super::{trim_path_token, DirectoryLookupInput, FileDeliveryLocatorInput};

// Pure locator parsing helpers shared by directory lookup and file delivery.
// Production callers start from structured locator hints or current-workspace
// contracts. Whole-request NL parsing lives in test support only.

pub(super) fn directory_lookup_input_from_hint(raw_hint: &str) -> Option<DirectoryLookupInput> {
    let hint = trim_path_token(raw_hint);
    if hint.is_empty() {
        return None;
    }
    if let Some(path) = extract_directory_path_candidate_from_request(&hint) {
        return Some(DirectoryLookupInput::ExplicitPath {
            directory_path: path,
        });
    }
    if looks_like_directory_path_hint(&hint) {
        return Some(DirectoryLookupInput::ExplicitPath {
            directory_path: hint,
        });
    }
    if is_compact_directory_hint_token(&hint) {
        return Some(DirectoryLookupInput::NameHint {
            directory_hint: hint,
        });
    }
    None
}

pub(super) fn extract_directory_path_candidate_from_request(text: &str) -> Option<String> {
    let explicit = extract_explicit_file_path_candidates(text);
    for token in explicit {
        if looks_like_explicit_directory_path_expression(&token)
            && explicit_path_prefers_directory_lookup(&token)
        {
            return Some(token);
        }
    }
    None
}

pub(super) fn classify_file_delivery_locator_from_hint(
    hint: &str,
) -> Option<FileDeliveryLocatorInput> {
    let raw = trim_path_token(hint);
    if raw.is_empty() {
        return None;
    }
    if let Some(path) = extract_definite_file_path_candidate(&raw) {
        return Some(FileDeliveryLocatorInput::ExplicitFilePath { file_path: path });
    }
    if let Some((directory_path, file_name)) = extract_structural_directory_and_file_pair_hint(&raw)
    {
        return Some(FileDeliveryLocatorInput::DirectoryAndFilename {
            directory_path,
            file_name,
        });
    }
    if looks_like_filename_token(&raw) {
        return Some(FileDeliveryLocatorInput::FilenameOnly { file_name: raw });
    }
    if looks_like_bare_filename_stem_token(&raw) {
        return Some(FileDeliveryLocatorInput::FilenameOnly { file_name: raw });
    }
    None
}

fn extract_structural_directory_and_file_pair_hint(text: &str) -> Option<(String, String)> {
    let filename_tokens = extract_filename_candidates(text);
    if filename_tokens.len() != 1 {
        return None;
    }
    extract_directory_path_candidate_for_file_pair(text)
        .map(|directory_path| (directory_path, filename_tokens[0].clone()))
}

pub(super) fn extract_explicit_file_path_candidates(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        Regex::new(r#"(?P<path>[A-Za-z]:\\[^\s"'`]+|(?:/|\.{1,2}/)[^\s"'`]+|[^\s"'`]+/[^\s"'`]+)"#)
            .expect("explicit path regex")
    });
    let mut out = Vec::new();
    for caps in regex.captures_iter(text) {
        let Some(m) = caps.name("path") else {
            continue;
        };
        let token = trim_path_token(m.as_str());
        if token.is_empty() || out.iter().any(|v| v == &token) {
            continue;
        }
        out.push(token);
    }
    out
}

pub(crate) fn extract_filename_candidates(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        Regex::new(
            r#"(?P<name>"[^"\n]+\.[A-Za-z0-9]{1,16}"|'[^'\n]+\.[A-Za-z0-9]{1,16}'|[^\s/\\,，、;；:：]+\.[A-Za-z0-9]{1,16})"#,
        )
        .expect("filename regex")
    });
    let mut out = Vec::new();
    for caps in regex.captures_iter(text) {
        let Some(m) = caps.name("name") else {
            continue;
        };
        let token = trim_path_token(m.as_str());
        if !looks_like_filename_token(&token)
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(&token)
            || out.iter().any(|v| v == &token)
        {
            continue;
        }
        out.push(token);
    }
    out
}

pub(super) fn looks_like_explicit_file_path_expression(path: &str) -> bool {
    let token = trim_path_token(path);
    if token.is_empty() || token.contains("://") || token.ends_with('/') || token.ends_with('\\') {
        return false;
    }
    let normalized = token.replace('\\', "/");
    if !normalized.contains('/') {
        return false;
    }
    let last = normalized.rsplit('/').next().unwrap_or_default();
    looks_like_filename_token(last)
}

pub(super) fn looks_like_explicit_directory_path_expression(path: &str) -> bool {
    let token = trim_path_token(path);
    if token.is_empty() || token.contains("://") {
        return false;
    }
    if token.ends_with('/') || token.ends_with('\\') {
        return true;
    }
    if token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || has_windows_drive_prefix(&token)
    {
        return !looks_like_explicit_file_path_expression(&token);
    }
    let normalized = token.replace('\\', "/");
    let last = normalized.rsplit('/').next().unwrap_or_default();
    normalized.contains('/') && !looks_like_filename_token(last)
}

pub(super) fn looks_like_directory_path_hint(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if cleaned.is_empty() || cleaned.contains("://") {
        return false;
    }
    if cleaned.ends_with('/') || cleaned.ends_with('\\') {
        return true;
    }
    let normalized = cleaned.replace('\\', "/");
    let last = normalized.rsplit('/').next().unwrap_or_default();
    if cleaned.starts_with('/')
        || cleaned.starts_with("./")
        || cleaned.starts_with("../")
        || has_windows_drive_prefix(&cleaned)
    {
        return !looks_like_filename_token(last);
    }
    normalized.contains('/') && !looks_like_filename_token(last)
}

pub(super) fn looks_like_filename_token(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if cleaned.is_empty()
        || cleaned.contains('\n')
        || cleaned.contains('\r')
        || cleaned.contains('\t')
        || cleaned.contains("://")
        || cleaned.starts_with("http://")
        || cleaned.starts_with("https://")
        || cleaned.contains('/')
        || cleaned.contains('\\')
    {
        return false;
    }
    let Some((base, ext)) = cleaned.rsplit_once('.') else {
        return false;
    };
    if base.trim().is_empty() || ext.is_empty() || ext.len() > 16 {
        return false;
    }
    if base.chars().count() > 128
        || base
            .chars()
            .all(|ch| ch.is_whitespace() || is_pure_punctuation(ch))
    {
        return false;
    }
    ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

pub(super) fn looks_like_bare_filename_stem_token(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if cleaned.is_empty()
        || cleaned.contains('\n')
        || cleaned.contains('\r')
        || cleaned.contains('\t')
        || cleaned.contains("://")
        || cleaned.contains('/')
        || cleaned.contains('\\')
        || looks_like_filename_token(&cleaned)
        || looks_like_directory_path_hint(&cleaned)
        || looks_like_explicit_directory_path_expression(&cleaned)
    {
        return false;
    }
    let chars = cleaned.chars().count();
    if chars < 2 || chars > 64 || !cleaned.is_ascii() {
        return false;
    }
    cleaned
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

pub(super) fn looks_like_directory_token(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if cleaned.is_empty()
        || cleaned.contains('\n')
        || cleaned.contains('\r')
        || cleaned.contains('\t')
        || cleaned.contains("://")
    {
        return false;
    }
    if cleaned.ends_with('/') || cleaned.ends_with('\\') {
        return true;
    }
    if cleaned.contains('/') || cleaned.contains('\\') {
        return !looks_like_explicit_file_path_expression(&cleaned);
    }
    if cleaned.starts_with('.') || cleaned.starts_with('~') {
        return true;
    }
    if looks_like_filename_token(&cleaned) {
        return false;
    }
    if cleaned.chars().count() > 128 {
        return false;
    }
    if cleaned.contains(char::is_whitespace) {
        return false;
    }
    !cleaned
        .chars()
        .all(|ch| ch.is_whitespace() || is_pure_punctuation(ch))
}

pub(super) fn is_pure_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ','
            | '，'
            | ';'
            | '；'
            | ':'
            | '：'
            | '!'
            | '！'
            | '？'
            | '。'
            | '('
            | ')'
            | '（'
            | '）'
            | '['
            | ']'
            | '【'
            | '】'
            | '{'
            | '}'
            | '「'
            | '」'
            | '《'
            | '》'
            | '"'
            | '\''
            | '`'
    )
}

pub(super) fn normalize_locator_text(text: &str) -> String {
    trim_path_token(text)
        .chars()
        .map(|ch| match ch {
            '／' | '、' => '/',
            '－' => '-',
            '＿' => '_',
            '。' => '.',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            '｛' => '{',
            '｝' => '}',
            '　' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .to_lowercase()
}

pub(super) fn has_windows_drive_prefix(token: &str) -> bool {
    token.len() >= 3
        && token.as_bytes()[0].is_ascii_alphabetic()
        && token.as_bytes()[1] == b':'
        && (token.as_bytes()[2] == b'\\' || token.as_bytes()[2] == b'/')
}

pub(super) fn extract_directory_path_candidate_for_file_pair(text: &str) -> Option<String> {
    extract_explicit_file_path_candidates(text)
        .into_iter()
        .find(|token| looks_like_explicit_directory_path_expression(token))
}

pub(super) fn extract_definite_file_path_candidate(text: &str) -> Option<String> {
    extract_explicit_file_path_candidates(text)
        .into_iter()
        .find(|token| looks_like_explicit_file_path_expression(token))
}

fn explicit_path_prefers_directory_lookup(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if cleaned.is_empty() {
        return false;
    }
    if cleaned.ends_with('/') || cleaned.ends_with('\\') {
        return true;
    }
    if looks_like_explicit_file_path_expression(&cleaned) {
        return false;
    }
    let normalized = cleaned.replace('\\', "/");
    let last = normalized.rsplit('/').next().unwrap_or_default();
    normalized.contains('/')
        && !last.is_empty()
        && !looks_like_filename_token(last)
        && last.chars().count() <= 4
}

fn is_compact_directory_hint_token(token: &str) -> bool {
    let cleaned = trim_path_token(token);
    if !looks_like_directory_token(&cleaned) {
        return false;
    }
    let chars = cleaned.chars().count();
    if chars < 2 || chars > 32 {
        return false;
    }
    if cleaned.is_ascii() {
        chars >= 3
            && cleaned
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    } else {
        true
    }
}
