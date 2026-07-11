use crate::OutputLocatorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructuredLocatorTokenKind {
    Path,
    Url,
    Filename,
    DeliveryToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructuredLocatorToken {
    pub(crate) kind: StructuredLocatorTokenKind,
    pub(crate) value: String,
    pub(crate) reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedLocator {
    pub(crate) locator_kind: OutputLocatorKind,
    pub(crate) locator_hint: String,
    pub(crate) reason: &'static str,
}

pub(crate) fn extract_explicit_locator_for_fallback(
    user_request: &str,
) -> Option<ExtractedLocator> {
    for token in structured_locator_tokens(user_request) {
        match token.kind {
            StructuredLocatorTokenKind::Path => {
                return Some(ExtractedLocator {
                    locator_kind: OutputLocatorKind::Path,
                    locator_hint: token.value,
                    reason: token.reason,
                });
            }
            StructuredLocatorTokenKind::Url => {
                return Some(ExtractedLocator {
                    locator_kind: OutputLocatorKind::Url,
                    locator_hint: token.value,
                    reason: token.reason,
                });
            }
            StructuredLocatorTokenKind::Filename | StructuredLocatorTokenKind::DeliveryToken => {}
        }
    }

    let mut filename_candidates = structured_locator_tokens(user_request)
        .into_iter()
        .filter(|token| token.kind == StructuredLocatorTokenKind::Filename)
        .map(|token| token.value)
        .collect::<Vec<_>>();
    if filename_candidates.len() == 1 {
        return Some(ExtractedLocator {
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: filename_candidates.remove(0),
            reason: "explicit_filename_locator",
        });
    }
    None
}

pub(crate) fn structured_locator_tokens(user_request: &str) -> Vec<StructuredLocatorToken> {
    let mut out = Vec::new();
    for locator in extract_explicit_locator_candidates_for_fallback(user_request) {
        let kind = match locator.locator_kind {
            OutputLocatorKind::Path => StructuredLocatorTokenKind::Path,
            OutputLocatorKind::Url => StructuredLocatorTokenKind::Url,
            _ => continue,
        };
        push_structured_locator_token(
            &mut out,
            StructuredLocatorToken {
                kind,
                value: locator.locator_hint,
                reason: locator.reason,
            },
        );
    }
    for filename in crate::delivery_utils::extract_filename_candidates(user_request) {
        if candidate_looks_like_dotted_version_number(&filename) {
            continue;
        }
        push_structured_locator_token(
            &mut out,
            StructuredLocatorToken {
                kind: StructuredLocatorTokenKind::Filename,
                value: filename,
                reason: "explicit_filename_locator",
            },
        );
    }
    for token in crate::extract_delivery_file_tokens(user_request) {
        push_structured_locator_token(
            &mut out,
            StructuredLocatorToken {
                kind: StructuredLocatorTokenKind::DeliveryToken,
                value: token,
                reason: "delivery_token_locator",
            },
        );
    }
    out
}

fn push_structured_locator_token(
    out: &mut Vec<StructuredLocatorToken>,
    token: StructuredLocatorToken,
) {
    if !out
        .iter()
        .any(|existing| existing.kind == token.kind && existing.value == token.value)
    {
        out.push(token);
    }
}

pub(crate) fn extract_explicit_locator_candidates_for_fallback(
    user_request: &str,
) -> Vec<ExtractedLocator> {
    let mut out = user_request
        .split_whitespace()
        .filter_map(|token| {
            let trimmed = trim_fallback_locator_token(token);
            if trimmed.is_empty() {
                return None;
            }
            let candidate = trimmed
                .split(|ch: char| {
                    matches!(
                        ch,
                        ',' | '，' | '。' | ';' | '；' | ')' | '）' | ']' | '}' | '>' | '》'
                    )
                })
                .next()
                .unwrap_or_default()
                .trim();
            (!candidate.is_empty()).then(|| candidate.to_string())
        })
        .filter_map(|candidate| classify_explicit_locator_candidate(&candidate))
        .collect::<Vec<_>>();
    out.dedup_by(|left, right| {
        left.locator_kind == right.locator_kind && left.locator_hint == right.locator_hint
    });
    out
}

fn classify_explicit_locator_candidate(candidate: &str) -> Option<ExtractedLocator> {
    if candidate_looks_like_dotted_version_number(candidate) {
        return None;
    }
    if candidate_looks_like_protocol_field_path(candidate) {
        return None;
    }
    if !crate::worker::has_explicit_path_or_url_locator_hint(candidate) {
        return None;
    }
    let lower = candidate.to_ascii_lowercase();
    let locator_kind = if lower.starts_with("http://") || lower.starts_with("https://") {
        OutputLocatorKind::Url
    } else {
        OutputLocatorKind::Path
    };
    Some(ExtractedLocator {
        locator_kind,
        locator_hint: candidate.to_string(),
        reason: match locator_kind {
            OutputLocatorKind::Url => "explicit_url_locator",
            OutputLocatorKind::Path => "explicit_path_locator",
            _ => "explicit_locator",
        },
    })
}

fn candidate_looks_like_protocol_field_path(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    if !trimmed.contains(['/', '\\']) || trimmed.starts_with(['/', '\\']) {
        return false;
    }
    let parts = trimmed
        .split(['/', '\\'])
        .map(str::trim)
        .collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() > 4 || parts.iter().any(|part| part.is_empty()) {
        return false;
    }
    parts.iter().all(|part| {
        part.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            && protocol_or_lifecycle_field_token(part)
    })
}

fn protocol_or_lifecycle_field_token(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "accepted"
            | "background"
            | "cancel_ref"
            | "cancelled"
            | "canceled"
            | "checkpoint_id"
            | "error_text"
            | "failed"
            | "machine_reply"
            | "needs_user"
            | "next_check_after"
            | "pending"
            | "poll_ref"
            | "queued"
            | "repairenvelope"
            | "resume_context"
            | "running"
            | "succeeded"
            | "task_id"
            | "task_lifecycle"
            | "text"
            | "timeout"
            | "waiting"
    )
}

pub(crate) fn candidate_looks_like_dotted_version_number(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    if trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    let mut parts = trimmed.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty() || !first.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let mut saw_dot_segment = false;
    for part in parts {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
        saw_dot_segment = true;
    }
    saw_dot_segment
}

fn trim_fallback_locator_token(token: &str) -> String {
    let mut trimmed = token
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
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        })
        .to_string();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.ends_with('.')
        && (trimmed.contains('/')
            || trimmed.contains('\\')
            || lower.starts_with("http://")
            || lower.starts_with("https://"))
    {
        trimmed.pop();
    }
    trimmed
}

#[cfg(test)]
#[path = "locator_extractor_tests.rs"]
mod tests;
