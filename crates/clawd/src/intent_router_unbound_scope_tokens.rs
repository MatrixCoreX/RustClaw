use super::*;

pub(super) fn token_is_unbound_scope_identifier(candidate: &str) -> bool {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
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
    });
    if trimmed.len() < 2
        || trimmed.len() > 128
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('.')
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return false;
    }
    let mut has_ascii_alnum = false;
    let mut has_scope_separator = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            has_ascii_alnum = true;
            continue;
        }
        if matches!(ch, '_' | '-') {
            has_scope_separator = true;
            continue;
        }
        return false;
    }
    has_ascii_alnum && has_scope_separator
}

pub(super) fn single_unbound_scope_identifier_outside_filename(
    prompt: &str,
    filename: &str,
) -> Option<String> {
    let mut matches = Vec::new();
    for token in prompt.split_whitespace().flat_map(|token| {
        token.split(|ch: char| matches!(ch, ',' | '，' | '。' | ';' | '；' | '、' | ':' | '：'))
    }) {
        let trimmed = token.trim();
        if trimmed.eq_ignore_ascii_case(filename) || !token_is_unbound_scope_identifier(trimmed) {
            continue;
        }
        if !matches
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            matches.push(trimmed.to_string());
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

pub(super) fn surface_has_unbound_scope_plus_single_filename_target(
    route_reason: &str,
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if !route_reason_has_machine_marker(route_reason, "existence_with_path")
        || output_contract.delivery_required
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
    {
        return false;
    }
    let filenames = req_surface.filename_candidates_excluding_field_selectors();
    if filenames.len() != 1 {
        return false;
    }
    single_unbound_scope_identifier_outside_filename(req, &filenames[0]).is_some()
}

fn route_reason_has_machine_marker(route_reason: &str, marker: &str) -> bool {
    route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
}
