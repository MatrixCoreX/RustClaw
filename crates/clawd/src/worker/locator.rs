/// Detects explicit path, URL, or filename syntax without interpreting the
/// surrounding natural-language request.
pub(crate) fn has_concrete_locator_hint(text: &str) -> bool {
    if has_explicit_path_or_url_locator(text) {
        return true;
    }
    text.split_whitespace()
        .flat_map(locator_token_segments)
        .any(|token| looks_like_filename_locator(&token))
}

pub(crate) fn has_explicit_path_or_url_locator_hint(text: &str) -> bool {
    has_explicit_path_or_url_locator(text)
}

fn trim_locator_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | ','
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | ')'
                    | '('
                    | ']'
                    | '['
                    | '）'
                    | '（'
                    | '】'
                    | '【'
                    | '>'
                    | '<'
                    | '》'
                    | '《'
            )
        })
        .trim_end_matches('.')
        .to_string()
}

fn locator_token_segments(token: &str) -> Vec<String> {
    token
        .split(|ch: char| matches!(ch, ',' | '，' | '。' | ';' | '；' | '、' | ':' | '：'))
        .map(trim_locator_token)
        .filter(|part| !part.is_empty())
        .collect()
}

fn has_explicit_path_or_url_locator(text: &str) -> bool {
    text.split_whitespace()
        .map(trim_locator_token)
        .any(|token| looks_like_explicit_path_or_url_token(&token))
}

fn looks_like_explicit_path_or_url_token(token: &str) -> bool {
    if token.is_empty() || looks_like_protocol_field_selector_path(token) {
        return false;
    }
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/")
        || token.starts_with("http://")
        || token.starts_with("https://")
        || token.contains(":\\")
        || (token.contains('/') && !token.contains("://"))
}

fn looks_like_protocol_field_selector_path(token: &str) -> bool {
    let trimmed = token.trim();
    if !trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.starts_with("~/")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.contains("://")
    {
        return false;
    }
    let segments = trimmed
        .split('/')
        .map(trim_locator_token)
        .collect::<Vec<_>>();
    segments.len() >= 2
        && segments
            .iter()
            .all(|segment| !segment.is_empty() && protocol_field_selector_segment(segment))
}

fn protocol_field_selector_segment(segment: &str) -> bool {
    let canonical = segment
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    matches!(
        canonical.as_str(),
        "args"
            | "checkpointid"
            | "context"
            | "decision"
            | "errorcode"
            | "errortext"
            | "extra"
            | "issuecode"
            | "issuecodes"
            | "messagekey"
            | "reasoncode"
            | "repairenvelope"
            | "repairsignal"
            | "requestid"
            | "status"
            | "statuscode"
            | "text"
    )
}

fn looks_like_filename_locator(token: &str) -> bool {
    if token.is_empty()
        || token.contains('/')
        || token.contains('\\')
        || token.starts_with("http://")
        || token.starts_with("https://")
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
    {
        return false;
    }
    let Some((base, extension)) = token.rsplit_once('.') else {
        return false;
    };
    !base.is_empty()
        && !extension.is_empty()
        && extension.len() <= 12
        && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
}

#[cfg(test)]
#[path = "locator_tests.rs"]
mod tests;
