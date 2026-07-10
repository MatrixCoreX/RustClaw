use std::sync::OnceLock;

use regex::Regex;

use super::locator::{
    classify_file_delivery_locator_from_hint, directory_lookup_input_from_hint,
    extract_definite_file_path_candidate, extract_directory_path_candidate_for_file_pair,
    extract_directory_path_candidate_from_request, extract_filename_candidates,
    looks_like_bare_filename_stem_token, looks_like_directory_path_hint,
    looks_like_directory_token, looks_like_filename_token, normalize_locator_text,
};
use super::{trim_path_token, DirectoryLookupInput, FileDeliveryLocatorInput};

pub(super) fn parse_directory_lookup_input_for_tests(text: &str) -> Option<DirectoryLookupInput> {
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

fn extract_directory_name_hint(text: &str) -> Option<String> {
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

pub(super) fn classify_file_delivery_locator_input_for_tests(
    user_request: &str,
    locator_hint: Option<&str>,
) -> Option<FileDeliveryLocatorInput> {
    if let Some(hint) = locator_hint.and_then(classify_file_delivery_locator_from_hint) {
        return Some(hint);
    }

    if let Some(file_path) = extract_definite_file_path_candidate(user_request) {
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

pub(super) fn extract_bare_filename_stem_candidates(text: &str) -> Vec<String> {
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

pub(super) fn extract_directory_and_file_pair(text: &str) -> Option<(String, String)> {
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
            if normalize_locator_text(&file) == normalize_locator_text(&directory_hint) {
                return None;
            }
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
