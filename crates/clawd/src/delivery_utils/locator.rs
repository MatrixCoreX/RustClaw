use std::sync::OnceLock;

use regex::Regex;

use super::{trim_path_token, DirectoryLookupInput, FileDeliveryLocatorInput};

// Pure locator parsing helpers shared by directory lookup and file delivery.
pub(super) fn parse_directory_lookup_input(text: &str) -> Option<DirectoryLookupInput> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || looks_like_directory_batch_file_request(trimmed)
        || extract_directory_and_file_pair(trimmed).is_some()
        || extract_filename_candidates(trimmed).len() == 1
        || extract_definite_file_path_candidate(trimmed).is_some()
    {
        return None;
    }
    if let Some(path) = extract_directory_path_candidate_from_request(trimmed) {
        return Some(DirectoryLookupInput::ExplicitPath {
            directory_path: path,
        });
    }

    if let Some(hint) = extract_directory_name_hint(trimmed) {
        return directory_lookup_input_from_hint(&hint);
    }
    None
}

fn looks_like_directory_batch_file_request(text: &str) -> bool {
    let normalized = normalize_locator_text(text);
    let mentions_directory = normalized.contains("目录")
        || normalized.contains("文件夹")
        || normalized.contains("directory")
        || normalized.contains("folder");
    let mentions_file_delivery = normalized.contains("发给我")
        || normalized.contains("发我")
        || normalized.contains("发送")
        || normalized.contains("send")
        || normalized.contains("文件路径")
        || normalized.contains("file path")
        || normalized.contains("filepath")
        || normalized.contains("path");
    mentions_directory && mentions_file_delivery
}

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

pub(super) fn extract_directory_name_hint(text: &str) -> Option<String> {
    static QUOTED_RE: OnceLock<Regex> = OnceLock::new();
    static BARE_DIR_RE: OnceLock<Regex> = OnceLock::new();

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
        if let Some(v) = directory_hint_from_structured_token(&raw) {
            return Some(v);
        }
    }

    let bare_dir = BARE_DIR_RE.get_or_init(|| {
        Regex::new(r#"(?i)(?P<hint>[^\s"'`/\\]{1,64})\s*(?:目录|文件夹|directory|folder|dir)\S*"#)
            .expect("bare directory hint regex")
    });
    for caps in bare_dir.captures_iter(text) {
        let Some(raw) = caps.name("hint").map(|m| trim_path_token(m.as_str())) else {
            continue;
        };
        if let Some(v) = directory_hint_from_structured_token(&raw) {
            return Some(v);
        }
    }

    None
}

fn directory_hint_from_structured_token(raw: &str) -> Option<String> {
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

    let explicit_file_path = extract_definite_file_path_candidate(user_request);
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
    let bare_stem_tokens = extract_bare_filename_stem_candidates(user_request);
    if bare_stem_tokens.len() == 1 {
        return Some(FileDeliveryLocatorInput::FilenameOnly {
            file_name: bare_stem_tokens[0].clone(),
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
    if let Some(path) = extract_definite_file_path_candidate(&raw) {
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

fn split_bare_stem_candidate_tokens<'a>(text: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    text.split_whitespace().flat_map(|token| {
        token.split(|ch: char| {
            matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '?'
                    | '？'
                    | '!'
                    | '！'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        })
    })
}

pub(crate) fn extract_bare_filename_stem_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in split_bare_stem_candidate_tokens(text) {
        let token = trim_path_token(raw);
        if !looks_like_bare_filename_stem_token(&token) || out.iter().any(|v| v == &token) {
            continue;
        }
        out.push(token);
    }
    out
}

pub(crate) fn extract_directory_and_file_pair(text: &str) -> Option<(String, String)> {
    let filename_tokens = extract_filename_candidates(text);
    let file = if filename_tokens.len() == 1 {
        filename_tokens[0].clone()
    } else {
        String::new()
    };
    let tokens = text
        .split_whitespace()
        .map(trim_path_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if file.is_empty() {
        if let Some(path) = extract_directory_path_candidate_for_file_pair(text) {
            let path_norm = normalize_locator_text(&path);
            let path_idx = tokens
                .iter()
                .position(|token| normalize_locator_text(token) == path_norm);
            let trailing_segment = tokens
                .iter()
                .skip(path_idx.map(|idx| idx + 1).unwrap_or(0))
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let bare_after_path = extract_bare_filename_stem_candidates(&trailing_segment)
                .into_iter()
                .filter(|token| !is_locator_context_stopword(token))
                .collect::<Vec<_>>();
            if bare_after_path.len() == 1 {
                return Some((path, bare_after_path[0].clone()));
            }
        }

        let bare_candidates = tokens
            .iter()
            .filter(|token| looks_like_bare_filename_stem_token(token))
            .cloned()
            .collect::<Vec<_>>();
        if bare_candidates.len() != 1 {
            return None;
        }
        let file = bare_candidates[0].clone();
        if let Some(directory_hint) = extract_directory_name_hint(text) {
            return Some((directory_hint, file));
        }
        return None;
    }

    if let Some(path) = extract_directory_path_candidate_for_file_pair(text) {
        return Some((path, file));
    }

    if let Some(directory_hint) = extract_directory_name_hint(text) {
        return Some((directory_hint, file));
    }

    None
}

fn is_locator_context_stopword(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "where"
            | "is"
            | "the"
            | "a"
            | "an"
            | "just"
            | "only"
            | "output"
            | "path"
            | "return"
            | "give"
            | "me"
            | "show"
            | "tell"
            | "find"
            | "locate"
            | "and"
            | "or"
            | "for"
            | "to"
            | "of"
            | "in"
            | "at"
            | "under"
            | "inside"
    )
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

fn extract_directory_path_candidate_for_file_pair(text: &str) -> Option<String> {
    extract_explicit_file_path_candidates(text)
        .into_iter()
        .find(|token| looks_like_explicit_directory_path_expression(token))
}

fn extract_definite_file_path_candidate(text: &str) -> Option<String> {
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
