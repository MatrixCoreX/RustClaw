use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use super::{
    candidate_path_from_root, dedup_and_sort_paths, has_windows_drive_prefix, is_pure_punctuation,
    trim_path_token, DeliveryMessageKind, DirectoryLookupInput, FileDeliveryLocatorInput,
};

pub(super) fn parse_directory_lookup_input(text: &str) -> Option<DirectoryLookupInput> {
    if let Some(path) = extract_directory_path_candidate_from_request(text) {
        return Some(DirectoryLookupInput::ExplicitPath { directory_path: path });
    }

    if let Some(hint) = extract_directory_name_hint(text) {
        return directory_lookup_input_from_hint(&hint);
    }
    None
}

pub(super) fn directory_lookup_input_from_hint(raw_hint: &str) -> Option<DirectoryLookupInput> {
    let hint = trim_path_token(raw_hint);
    if hint.is_empty() {
        return None;
    }
    if let Some(path) = extract_directory_path_candidate_from_request(&hint) {
        return Some(DirectoryLookupInput::ExplicitPath { directory_path: path });
    }
    if looks_like_directory_path_hint(&hint) {
        return Some(DirectoryLookupInput::ExplicitPath { directory_path: hint });
    }
    if looks_like_directory_token(&hint) {
        return Some(DirectoryLookupInput::NameHint { directory_hint: hint });
    }
    None
}

pub(super) fn extract_directory_path_candidate_from_request(text: &str) -> Option<String> {
    let explicit = extract_explicit_file_path_candidates(text);
    for token in explicit {
        if looks_like_explicit_directory_path_expression(&token) {
            return Some(token);
        }
    }
    None
}

pub(super) fn extract_directory_name_hint(text: &str) -> Option<String> {
    static QUOTED_RE: OnceLock<Regex> = OnceLock::new();

    if let Some(path) = extract_directory_path_candidate_from_request(text) {
        return Some(path);
    }

    let quoted = QUOTED_RE.get_or_init(|| {
        Regex::new(r#"(?P<q>"[^"\n]+"|'[^'\n]+'|`[^`\n]+`)"#).expect("quoted dir hint regex")
    });
    for caps in quoted.captures_iter(text) {
        let Some(raw) = caps.name("q").map(|m| trim_path_token(m.as_str())) else {
            continue;
        };
        if let Some(v) = directory_hint_from_token(&raw) {
            return Some(v);
        }
    }

    let tokens = text
        .split_whitespace()
        .map(trim_path_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.len() == 1 {
        return directory_hint_from_token(&tokens[0]);
    }

    let mut candidates = Vec::new();
    for token in tokens {
        if let Some(v) = directory_hint_from_token(&token) {
            if !candidates.iter().any(|existing| existing == &v) {
                candidates.push(v);
            }
        }
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    let path_like = candidates
        .into_iter()
        .filter(|v| looks_like_directory_path_hint(v))
        .collect::<Vec<_>>();
    if path_like.len() == 1 {
        return path_like.into_iter().next();
    }
    None
}

pub(super) fn directory_hint_from_token(raw: &str) -> Option<String> {
    let token = trim_path_token(raw);
    if token.is_empty() || looks_like_filename_token(&token) {
        return None;
    }
    if looks_like_directory_path_hint(&token) || looks_like_directory_token(&token) {
        return Some(token);
    }
    None
}

pub(super) fn classify_file_delivery_locator_input(
    user_request: &str,
    locator_hint: Option<&str>,
) -> Option<FileDeliveryLocatorInput> {
    if let Some(hint) = locator_hint.and_then(classify_file_delivery_locator_from_hint) {
        return Some(hint);
    }

    let explicit_file_path = extract_explicit_file_path_candidates(user_request)
        .into_iter()
        .find(|token| looks_like_explicit_file_path_expression(token));
    if let Some(file_path) = explicit_file_path {
        return Some(FileDeliveryLocatorInput::ExplicitFilePath { file_path });
    }

    if let Some((directory_path, file_name)) = extract_directory_and_file_pair(user_request) {
        return Some(FileDeliveryLocatorInput::DirectoryAndFilename {
            directory_path,
            file_name,
        });
    }

    let filename_tokens = extract_filename_candidates(user_request);
    if filename_tokens.len() == 1 {
        return Some(FileDeliveryLocatorInput::FilenameOnly {
            file_name: filename_tokens[0].clone(),
        });
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
    if let Some(path) = extract_explicit_file_path_candidates(&raw)
        .into_iter()
        .find(|token| looks_like_explicit_file_path_expression(token))
    {
        return Some(FileDeliveryLocatorInput::ExplicitFilePath { file_path: path });
    }
    if let Some((directory_path, file_name)) = extract_directory_and_file_pair(&raw) {
        return Some(FileDeliveryLocatorInput::DirectoryAndFilename {
            directory_path,
            file_name,
        });
    }
    if looks_like_filename_token(&raw) {
        return Some(FileDeliveryLocatorInput::FilenameOnly { file_name: raw });
    }
    None
}

pub(super) fn extract_explicit_file_path_candidates(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        Regex::new(
            r#"(?P<path>[A-Za-z]:\\[^\s"'`]+|(?:/|\.{1,2}/)[^\s"'`]+|[^\s"'`]+/[^\s"'`]+)"#,
        )
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

pub(super) fn extract_filename_candidates(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| {
        Regex::new(
            r#"(?P<name>"[^"\n]+?\.[A-Za-z0-9]{1,16}"|'[^'\n]+?\.[A-Za-z0-9]{1,16}'|[^\s/\\]+?\.[A-Za-z0-9]{1,16})"#,
        )
        .expect("filename regex")
    });
    let mut out = Vec::new();
    for caps in regex.captures_iter(text) {
        let Some(m) = caps.name("name") else {
            continue;
        };
        let token = trim_path_token(m.as_str());
        if !looks_like_filename_token(&token) || out.iter().any(|v| v == &token) {
            continue;
        }
        out.push(token);
    }
    out
}

pub(super) fn extract_directory_and_file_pair(text: &str) -> Option<(String, String)> {
    let filename_tokens = extract_filename_candidates(text);
    if filename_tokens.len() != 1 {
        return None;
    }
    let file = filename_tokens[0].clone();
    let file_norm = normalize_locator_text(&file);

    if let Some(path) = extract_directory_path_candidate_from_request(text) {
        return Some((path, file));
    }

    let tokens = text
        .split_whitespace()
        .map(trim_path_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    let file_idx = tokens
        .iter()
        .position(|token| normalize_locator_text(token) == file_norm)?;

    for idx in (0..file_idx).rev() {
        let token = &tokens[idx];
        if looks_like_directory_token(token) {
            return Some((token.clone(), file));
        }
    }
    for token in tokens.iter().skip(file_idx + 1) {
        if looks_like_directory_token(token) {
            return Some((token.clone(), file));
        }
    }

    None
}

pub(super) fn looks_like_explicit_file_path_expression(path: &str) -> bool {
    let token = trim_path_token(path);
    if token.is_empty() || token.contains("://") || token.ends_with('/') || token.ends_with('\\') {
        return false;
    }
    if token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || has_windows_drive_prefix(&token)
    {
        return true;
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

pub(super) fn dedup_and_sort_paths(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    paths.dedup();
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
        || base.chars().all(|ch| ch.is_whitespace() || is_pure_punctuation(ch))
    {
        return false;
    }
    ext.chars().all(|ch| ch.is_ascii_alphanumeric())
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
        '.'
            | ','
            | '?'
            | ';'
            | '?'
            | ':'
            | '?'
            | '!'
            | '?'
            | '?'
            | '?'
            | '('
            | ')'
            | '?'
            | '?'
            | '['
            | ']'
            | '?'
            | '?'
            | '{'
            | '}'
            | '?'
            | '?'
            | '?'
            | '?'
            | '"'
            | '\''
            | '`'
    )
}

pub(super) fn normalize_locator_text(text: &str) -> String {
    trim_path_token(text)
        .chars()
        .map(|ch| match ch {
            '?' | '?' => '/',
            '?' => '-',
            '?' => '_',
            '?' => '.',
            '?' => '(',
            '?' => ')',
            '?' => '[',
            '?' => ']',
            '?' => '{',
            '?' => '}',
            '?' => ' ',
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

pub(super) fn resolve_existing_file_under_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let candidate = candidate_path_from_root(root, raw_path)?;
    let canonical = candidate.canonicalize().ok()?;
    canonical.is_file().then_some(canonical)
}

pub(super) fn resolve_existing_dir_under_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let candidate = candidate_path_from_root(root, raw_path)?;
    let canonical = candidate.canonicalize().ok()?;
    canonical.is_dir().then_some(canonical)
}

pub(super) fn candidate_path_from_root(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let cleaned = trim_path_token(raw_path);
    if cleaned.is_empty() {
        return None;
    }
    if has_windows_drive_prefix(&cleaned) {
        return Some(PathBuf::from(cleaned));
    }
    let relative = cleaned
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_string();
    if relative.is_empty() {
        return Some(root.to_path_buf());
    }
    Some(root.join(relative))
}
