use crate::OutputLocatorKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedLocator {
    pub(crate) locator_kind: OutputLocatorKind,
    pub(crate) locator_hint: String,
    pub(crate) reason: &'static str,
}

pub(crate) fn extract_explicit_locator_for_fallback(
    user_request: &str,
) -> Option<ExtractedLocator> {
    let explicit_path_or_url = extract_explicit_locator_candidates_for_fallback(user_request)
        .into_iter()
        .next();
    if explicit_path_or_url.is_some() {
        return explicit_path_or_url;
    }

    let mut filename_candidates =
        crate::intent::surface_signals::analyze_prompt_surface(user_request)
            .filename_candidates_excluding_field_selectors();
    filename_candidates.retain(|candidate| !candidate_looks_like_dotted_version_number(candidate));
    filename_candidates.sort();
    if filename_candidates.len() == 1 {
        return Some(ExtractedLocator {
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: filename_candidates.remove(0),
            reason: "explicit_filename_locator",
        });
    }
    None
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

fn candidate_looks_like_dotted_version_number(candidate: &str) -> bool {
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
    token
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
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        extract_explicit_locator_candidates_for_fallback, extract_explicit_locator_for_fallback,
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
}
