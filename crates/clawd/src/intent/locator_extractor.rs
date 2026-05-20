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
            let lower = trimmed.to_ascii_lowercase();
            if (lower.starts_with("http://") || lower.starts_with("https://"))
                && crate::worker::has_explicit_path_or_url_locator_hint(&trimmed)
            {
                return Some(trimmed);
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
mod tests {
    use super::{
        extract_explicit_locator_candidates_for_fallback, extract_explicit_locator_for_fallback,
        structured_locator_tokens, StructuredLocatorTokenKind,
    };
    use crate::OutputLocatorKind;

    #[test]
    fn extracts_relative_path_locator_from_mixed_text() {
        let out = extract_explicit_locator_for_fallback(
            "看一下 scripts/nl_tests/fixtures/device_local/configs/app_config.toml，然后用一句大白话说它主要配置了什么",
        )
        .expect("path locator should be extracted");
        assert_eq!(out.locator_kind, OutputLocatorKind::Path);
        assert_eq!(
            out.locator_hint,
            "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
        );
        assert_eq!(out.reason, "explicit_path_locator");
    }

    #[test]
    fn strips_terminal_sentence_period_from_path_locator() {
        let out = extract_explicit_locator_for_fallback(
            "Remember that the note file means scripts/nl_tests/fixtures/device_local/docs/service_notes.md.",
        )
        .expect("path locator should be extracted");

        assert_eq!(out.locator_kind, OutputLocatorKind::Path);
        assert_eq!(
            out.locator_hint,
            "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
        );
    }

    #[test]
    fn extracts_url_locator_without_downgrading_to_path() {
        let out = extract_explicit_locator_for_fallback(
            "请求一下 http://127.0.0.1:8787/v1/health ，如果能通就简短总结结果",
        )
        .expect("url locator should be extracted");
        assert_eq!(out.locator_kind, OutputLocatorKind::Url);
        assert_eq!(out.locator_hint, "http://127.0.0.1:8787/v1/health");
        assert_eq!(out.reason, "explicit_url_locator");
    }

    #[test]
    fn ignores_non_locator_tokens() {
        assert!(extract_explicit_locator_for_fallback("给我讲个笑话").is_none());
    }

    #[test]
    fn ignores_python_version_numbers_as_path_locators() {
        assert!(extract_explicit_locator_for_fallback(
            "Correction: not Python 3.10, use Python 3.11"
        )
        .is_none());
    }

    #[test]
    fn extracts_filename_locator_from_mixed_delivery_text() {
        let out = extract_explicit_locator_for_fallback("把 README.md 发给我")
            .expect("filename locator should be extracted");
        assert_eq!(out.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(out.locator_hint, "README.md");
        assert_eq!(out.reason, "explicit_filename_locator");
    }

    #[test]
    fn extracts_multiple_explicit_path_locators_from_mixed_text() {
        let out = extract_explicit_locator_candidates_for_fallback(
            "读一下 /tmp/a.md 的开头，然后顺手说 /tmp/b.md 是干什么的",
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].locator_kind, OutputLocatorKind::Path);
        assert_eq!(out[0].locator_hint, "/tmp/a.md");
        assert_eq!(out[1].locator_kind, OutputLocatorKind::Path);
        assert_eq!(out[1].locator_hint, "/tmp/b.md");
    }

    #[test]
    fn structured_locator_tokens_keep_only_structural_locator_shapes() {
        let out = structured_locator_tokens(
            "read docs/report.md and README.md, but not README\nFILE:/tmp/out.txt",
        );
        assert!(out
            .iter()
            .any(|token| token.kind == StructuredLocatorTokenKind::Path
                && token.value == "docs/report.md"));
        assert!(out
            .iter()
            .any(|token| token.kind == StructuredLocatorTokenKind::Filename
                && token.value == "README.md"));
        assert!(out
            .iter()
            .any(|token| token.kind == StructuredLocatorTokenKind::DeliveryToken));
        assert!(!out.iter().any(|token| token.value == "README"));
    }
}
