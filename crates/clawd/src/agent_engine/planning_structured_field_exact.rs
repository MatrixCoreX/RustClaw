use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

pub(in crate::agent_engine) fn extract_exact_structured_field_path_selectors(
    path: &str,
    text: &str,
) -> Vec<String> {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return Vec::new();
    };
    static DOTTED_SELECTOR_RE: OnceLock<Regex> = OnceLock::new();
    let re = DOTTED_SELECTOR_RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z_$][A-Za-z0-9_$-]*(?:\.[A-Za-z_$][A-Za-z0-9_$-]*)+\b")
            .expect("valid dotted selector regex")
    });
    let mut out = Vec::new();
    for candidate in re.find_iter(text) {
        let token = candidate.as_str().trim();
        if selector_candidate_is_valid(token)
            && lookup_structured_field_value(&value, token).is_some()
            && !out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(token))
        {
            out.push(token.to_string());
        }
    }
    out
}

fn selector_candidate_is_valid(candidate: &str) -> bool {
    let token = candidate.trim();
    !token.is_empty()
        && token.contains('.')
        && !token.contains('/')
        && !token.contains('\\')
        && !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        && token.split('.').all(segment_is_valid)
}

fn segment_is_valid(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
}

fn parse_structured_file_value(path: &Path) -> Option<Value> {
    let contents = fs::read_to_string(path).ok()?;
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => serde_json::from_str(&contents).ok(),
        "toml" => toml::from_str::<toml::Value>(&contents)
            .ok()
            .and_then(|value| serde_json::to_value(value).ok()),
        _ => None,
    }
}

fn lookup_structured_field_value<'a>(value: &'a Value, field_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in field_path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}
