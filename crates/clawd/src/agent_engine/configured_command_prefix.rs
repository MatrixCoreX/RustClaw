use super::*;

pub(super) fn strip_configured_command_prefix<'a>(
    request: &'a str,
    prefix: &str,
) -> Option<&'a str> {
    let request = request.trim_start();
    let prefix = prefix.trim_start();
    if request.is_empty() || prefix.is_empty() {
        return None;
    }
    if prefix.is_ascii() {
        let request_lower = request.to_ascii_lowercase();
        let prefix_lower = prefix.to_ascii_lowercase();
        request_lower
            .starts_with(&prefix_lower)
            .then(|| &request[prefix.len()..])
    } else {
        request
            .starts_with(prefix)
            .then(|| &request[prefix.len()..])
    }
}

pub(super) fn trim_leading_command_delimiters(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        let Some(ch) = text.chars().next() else {
            return text;
        };
        if matches!(
            ch,
            ':' | '：' | '-' | '—' | '–' | '`' | '"' | '\'' | '“' | '”' | ' '
        ) {
            text = &text[ch.len_utf8()..];
            continue;
        }
        return text;
    }
}

pub(super) fn trim_leading_command_separators_preserve_quotes(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        let Some(ch) = text.chars().next() else {
            return text;
        };
        if matches!(ch, ':' | '：' | '-' | '—' | '–' | ' ') {
            text = &text[ch.len_utf8()..];
            continue;
        }
        return text;
    }
}

pub(super) fn looks_like_concrete_command_tail(tail: &str) -> bool {
    let tail = trim_leading_command_delimiters(tail);
    let first_token = tail
        .split_whitespace()
        .next()
        .unwrap_or(tail)
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation()
                || matches!(ch, '，' | '。' | '；' | '：' | '、' | '！' | '？')
        });
    first_token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count()
        >= 2
}

pub(super) fn explicit_command_segment_before_followup(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail);
    let boundary = tail.char_indices().find_map(|(idx, ch)| {
        (idx > 0 && matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n')).then_some(idx)
    })?;
    Some(&tail[..boundary])
}

pub(super) fn explicit_command_followup_tail(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail);
    let boundary = tail.char_indices().find_map(|(idx, ch)| {
        (idx > 0 && matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n')).then_some(idx)
    })?;
    let delimiter_len = tail[boundary..]
        .chars()
        .next()
        .map(char::len_utf8)
        .unwrap_or(0);
    Some(tail[boundary + delimiter_len..].trim())
}

pub(super) fn whole_explicit_command_tail(tail: &str) -> Option<&str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }
    if tail
        .chars()
        .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return Some(tail);
    }
    let mut tokens = tail.split_whitespace();
    let first = tokens.next()?;
    if tokens.clone().next().is_none() {
        return Some(first);
    }
    tokens
        .all(structural_command_argument_token)
        .then_some(tail)
}

pub(super) fn markdown_code_span_command_segment(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text.strip_prefix('`')?;
    let close = rest.find('`')?;
    let command = rest[..close].trim();
    if command.is_empty() {
        return None;
    }
    let suffix = rest[close + '`'.len_utf8()..].trim();
    if suffix.chars().all(|ch| {
        matches!(
            ch,
            '.' | '。' | '!' | '！' | '?' | '？' | ',' | '，' | ';' | '；'
        )
    }) {
        Some(command)
    } else {
        None
    }
}

pub(super) fn embedded_markdown_code_span_command_segment(text: &str) -> Option<&str> {
    let text = text.trim();
    let open = text.find('`')?;
    let rest = &text[open + '`'.len_utf8()..];
    let close = rest.find('`')?;
    let command = rest[..close].trim();
    if command.is_empty() || literal_command_segment_has_unresolved_template(command) {
        return None;
    }
    Some(command)
}

pub(super) fn structural_command_argument_token(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| {
        ch.is_ascii_punctuation() && !matches!(ch, '-' | '_' | '.' | '/' | '\\' | '~' | '=')
    });
    if token.is_empty() {
        return false;
    }
    let quoted = (token.starts_with('"') && token.ends_with('"'))
        || (token.starts_with('\'') && token.ends_with('\''));
    let flag = token.starts_with('-') && token.chars().any(|ch| ch.is_ascii_alphanumeric());
    let path_like = token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/")
        || token.contains('/')
        || token.contains('\\')
        || token.contains('.');
    let assignment = token
        .split_once('=')
        .is_some_and(|(name, value)| !name.is_empty() && !value.is_empty());
    let machine_literal = token.is_ascii()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && (token.contains('_') || token.chars().any(|ch| ch.is_ascii_digit()));
    quoted || flag || path_like || assignment || machine_literal
}

pub(super) fn configured_standalone_command_token_value<'a>(
    runtime: &'a crate::CommandIntentRuntime,
    token: &str,
) -> Option<&'a str> {
    runtime.standalone_commands.iter().find_map(|candidate| {
        if candidate.is_ascii() && token.is_ascii() {
            candidate
                .eq_ignore_ascii_case(token)
                .then_some(candidate.as_str())
        } else {
            (candidate == token).then_some(candidate.as_str())
        }
    })
}

pub(super) fn configured_standalone_command_token(
    runtime: &crate::CommandIntentRuntime,
    token: &str,
) -> bool {
    configured_standalone_command_token_value(runtime, token).is_some()
}

pub(super) fn command_candidate_end_boundary(text: &str, end_idx: usize) -> bool {
    if end_idx >= text.len() {
        return true;
    }
    let Some(next) = text[end_idx..].chars().next() else {
        return true;
    };
    !next.is_ascii_alphanumeric() && !matches!(next, '_' | '-' | '/' | '\\' | '~' | '`')
}

pub(super) fn configured_standalone_command_sequence_from_segment(
    runtime: &crate::CommandIntentRuntime,
    segment: &str,
) -> Option<String> {
    let segment = trim_leading_command_separators_preserve_quotes(segment).trim();
    if segment.is_empty()
        || segment.contains('\n')
        || segment.contains('`')
        || segment
            .chars()
            .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return None;
    }

    let mut commands = Vec::new();
    for (idx, ch) in segment.char_indices() {
        if !ch.is_ascii_alphabetic() || !command_candidate_start_boundary(segment, idx) {
            continue;
        }
        let mut end = idx;
        for (offset, candidate) in segment[idx..].char_indices() {
            if candidate.is_ascii_alphanumeric() || matches!(candidate, '_' | '-') {
                end = idx + offset + candidate.len_utf8();
                continue;
            }
            break;
        }
        if end <= idx || !command_candidate_end_boundary(segment, end) {
            continue;
        }
        let token = &segment[idx..end];
        if !simple_bare_command_token(token) {
            continue;
        }
        if let Some(canonical) = configured_standalone_command_token_value(runtime, token) {
            commands.push(canonical.to_string());
        }
    }

    (commands.len() >= 2).then(|| commands.join("; "))
}

#[cfg(test)]
pub(super) fn configured_distinct_standalone_command_sequence_from_text(
    runtime: &crate::CommandIntentRuntime,
    text: &str,
) -> Option<String> {
    let command = configured_standalone_command_sequence_from_segment(runtime, text)?;
    let commands = command
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let distinct = commands
        .iter()
        .map(|command| command.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    (distinct.len() >= 2).then_some(command)
}

pub(super) fn standalone_command_segment_before_freeform_tail<'a>(
    runtime: &crate::CommandIntentRuntime,
    tail: &'a str,
) -> Option<&'a str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }

    let mut tokens = tail.split_whitespace();
    let first = tokens.next()?;
    let first_start = tail.find(first)?;
    let first_end = first_start + first.len();
    let first_token =
        first.trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
    if !simple_bare_command_token(first_token)
        || !configured_standalone_command_token(runtime, first_token)
    {
        return None;
    }
    let mut end = first_end;
    let mut search_from = first_end;

    for raw_token in tokens {
        let token_start = tail[search_from..].find(raw_token)? + search_from;
        let token_end = token_start + raw_token.len();
        if structural_command_argument_token(raw_token) {
            end = token_end;
            search_from = token_end;
            continue;
        }
        return Some(tail[..end].trim());
    }

    None
}

pub(super) fn path_command_segment_before_freeform_tail<'a>(tail: &'a str) -> Option<&'a str> {
    let path_env = std::env::var_os("PATH");
    path_command_segment_before_freeform_tail_with_path_env(tail, path_env.as_deref())
}

pub(super) fn path_command_segment_before_freeform_tail_with_path_env<'a>(
    tail: &'a str,
    path_env: Option<&std::ffi::OsStr>,
) -> Option<&'a str> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }

    let mut tokens = tail.split_whitespace();
    let first = tokens.next()?;
    let first_start = tail.find(first)?;
    let first_end = first_start + first.len();
    let first_token =
        first.trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
    if !simple_bare_command_token(first_token)
        || !command_token_resolves_in_path(first_token, path_env)
    {
        return None;
    }

    let mut end = first_end;
    let mut search_from = first_end;
    let mut saw_structural_arg = false;
    for raw_token in tokens {
        let token_start = tail[search_from..].find(raw_token)? + search_from;
        let token_end = token_start + raw_token.len();
        if structural_command_argument_token(raw_token) {
            saw_structural_arg = true;
            end = token_end;
            search_from = token_end;
            continue;
        }
        return saw_structural_arg.then(|| tail[..end].trim());
    }

    saw_structural_arg.then(|| tail[..end].trim())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExplicitCommandCandidate {
    command: String,
    single_step_safe: bool,
}

pub(super) fn standalone_structural_command_from_segment(
    runtime: &crate::CommandIntentRuntime,
    segment: &str,
) -> Option<String> {
    let segment = trim_leading_command_separators_preserve_quotes(segment).trim();
    if segment.is_empty() || segment.contains('\n') || segment.contains('`') {
        return None;
    }
    let mut tokens = segment.split_whitespace();
    let first = tokens.next()?;
    let first_token =
        first.trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
    if !simple_bare_command_token(first_token)
        || !configured_standalone_command_token(runtime, first_token)
    {
        return None;
    }
    if !tokens.all(structural_command_argument_token) {
        return None;
    }
    let command = crate::bootstrap::config_loaders::trim_command_text(segment.to_string());
    (!command.is_empty()).then_some(command)
}

pub(super) fn followup_tail_has_structured_command_payload(
    runtime: &crate::CommandIntentRuntime,
    followup: &str,
) -> bool {
    let followup = followup.trim();
    !followup.is_empty()
        && (configured_explicit_command_candidate(runtime, followup).is_some()
            || embedded_configured_explicit_command_candidate(runtime, followup).is_some()
            || embedded_standalone_command_candidate(runtime, followup).is_some()
            || shellish_literal_command_segment(followup).is_some()
            || leading_shellish_command_sequence_segment(followup).is_some())
}

pub(super) fn standalone_command_candidate_from_tail(
    runtime: &crate::CommandIntentRuntime,
    tail: &str,
) -> Option<ExplicitCommandCandidate> {
    let tail = trim_leading_command_separators_preserve_quotes(tail).trim();
    if tail.is_empty() || tail.contains('\n') {
        return None;
    }

    if let Some(segment) = explicit_command_segment_before_followup(tail) {
        let command = configured_standalone_command_sequence_from_segment(runtime, segment)
            .or_else(|| standalone_structural_command_from_segment(runtime, segment))?;
        let followup = explicit_command_followup_tail(tail).unwrap_or("");
        return Some(ExplicitCommandCandidate {
            command,
            single_step_safe: !followup_tail_has_structured_command_payload(runtime, followup),
        });
    }

    if let Some(segment) = standalone_command_segment_before_freeform_tail(runtime, tail) {
        let command = standalone_structural_command_from_segment(runtime, segment)?;
        let followup = tail.get(segment.len()..).unwrap_or_default();
        return Some(ExplicitCommandCandidate {
            command,
            single_step_safe: !followup_tail_has_structured_command_payload(runtime, followup),
        });
    }

    let command = standalone_structural_command_from_segment(runtime, tail)?;
    Some(ExplicitCommandCandidate {
        command,
        single_step_safe: true,
    })
}

pub(super) fn command_candidate_start_boundary(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let Some(prev) = text[..idx].chars().next_back() else {
        return true;
    };
    !prev.is_ascii_alphanumeric() && !matches!(prev, '_' | '-' | '.' | '/' | '\\' | '~' | '`')
}

pub(super) fn embedded_standalone_command_candidate(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<ExplicitCommandCandidate> {
    let request = request.trim();
    if request.is_empty() {
        return None;
    }
    request
        .char_indices()
        .filter(|(idx, ch)| {
            ch.is_ascii_alphabetic() && command_candidate_start_boundary(request, *idx)
        })
        .filter_map(|(idx, _)| standalone_command_candidate_from_tail(runtime, &request[idx..]))
        .next()
}

pub(super) fn embedded_configured_explicit_command_candidate(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<ExplicitCommandCandidate> {
    let request = request.trim();
    if request.is_empty() {
        return None;
    }
    request
        .char_indices()
        .filter(|(idx, _)| command_candidate_start_boundary(request, *idx))
        .filter_map(|(idx, _)| {
            configured_explicit_command_candidate_from_text(runtime, &request[idx..], true)
        })
        .next()
}

pub(super) fn configured_explicit_command_candidate_from_text(
    runtime: &crate::CommandIntentRuntime,
    text: &str,
    allow_whole_tail: bool,
) -> Option<ExplicitCommandCandidate> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    runtime
        .execute_prefixes
        .iter()
        .filter_map(|prefix| strip_configured_command_prefix(text, prefix))
        .filter_map(|tail| {
            let segment = explicit_command_segment_before_followup(tail).or_else(|| {
                allow_whole_tail.then(|| {
                    markdown_code_span_command_segment(tail)
                        .or_else(|| embedded_markdown_code_span_command_segment(tail))
                        .or_else(|| whole_explicit_command_tail(tail))
                        .or_else(|| standalone_command_segment_before_freeform_tail(runtime, tail))
                        .or_else(|| path_command_segment_before_freeform_tail(tail))
                })?
            })?;
            let segment = markdown_code_span_command_segment(segment)
                .or_else(|| embedded_markdown_code_span_command_segment(segment))
                .unwrap_or(segment);
            let command = configured_standalone_command_sequence_from_segment(runtime, segment)
                .unwrap_or_else(|| {
                    crate::bootstrap::config_loaders::trim_command_text(segment.to_string())
                });
            if command.contains('`') || literal_command_segment_has_unresolved_template(&command) {
                return None;
            }
            let freeform_followup = tail.get(segment.len()..).unwrap_or_default();
            looks_like_concrete_command_tail(&command).then(|| ExplicitCommandCandidate {
                command,
                single_step_safe: explicit_command_followup_tail(tail).map_or_else(
                    || !followup_tail_has_structured_command_payload(runtime, freeform_followup),
                    |followup| !followup_tail_has_structured_command_payload(runtime, followup),
                ),
            })
        })
        .next()
}

pub(super) fn configured_explicit_command_candidate(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<ExplicitCommandCandidate> {
    let request = request.trim();
    if request.is_empty() {
        return None;
    }
    configured_explicit_command_candidate_from_text(runtime, request, true).or_else(|| {
        request
            .split(|ch| matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n'))
            .filter_map(|clause| {
                configured_explicit_command_candidate_from_text(runtime, clause, true)
            })
            .next()
    })
}

pub(super) fn configured_explicit_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    configured_explicit_command_candidate(runtime, request).map(|candidate| candidate.command)
}

pub(super) fn contains_angle_placeholder_token(text: &str) -> bool {
    let mut chars = text.char_indices().peekable();
    while let Some((start_idx, ch)) = chars.next() {
        if ch != '<' {
            continue;
        }
        let Some((end_idx, _)) = chars.clone().find(|(_, candidate)| *candidate == '>') else {
            continue;
        };
        let inner = text[start_idx + ch.len_utf8()..end_idx].trim();
        if inner.is_empty() {
            continue;
        }
        let has_identifier_char = inner.chars().any(|candidate| candidate.is_alphanumeric());
        let placeholder_shaped = inner.chars().all(|candidate| {
            candidate.is_alphanumeric() || matches!(candidate, '_' | '-' | '.' | ' ' | '\t')
        });
        if has_identifier_char && placeholder_shaped {
            return true;
        }
    }
    false
}

pub(super) fn literal_command_segment_has_unresolved_template(segment: &str) -> bool {
    contains_angle_placeholder_token(segment) || literal_segment_looks_like_output_template(segment)
}

pub(super) fn literal_segment_looks_like_output_template(segment: &str) -> bool {
    let segment = segment.trim();
    if segment.is_empty()
        || segment.contains('\n')
        || segment
            .chars()
            .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return false;
    }
    let mut words = segment.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let Some(rest) = words.next() else {
        return false;
    };
    if words.next().is_some() || !first.ends_with(':') {
        return false;
    }
    let label = first.trim_end_matches(':');
    let label_ok = !label.is_empty()
        && label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    let placeholder_ok = rest
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '<' | '>'));
    label_ok && placeholder_ok
}

pub(super) fn shellish_literal_command_segments(
    request: &str,
    allow_bare_token: bool,
) -> Vec<String> {
    let mut parts = request.split('`');
    parts.next();
    parts
        .step_by(2)
        .map(|segment| crate::bootstrap::config_loaders::trim_command_text(segment.to_string()))
        .filter(|segment| {
            !literal_command_segment_has_unresolved_template(segment)
                && looks_like_concrete_command_tail(segment)
                && (allow_bare_token
                    || segment
                        .chars()
                        .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<') || ch.is_whitespace()))
        })
        .collect()
}

pub(super) fn prefixed_shellish_command_segments(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
    allow_bare_token: bool,
) -> Vec<String> {
    request
        .split(|ch| matches!(ch, ',' | '，' | ';' | '；' | '。' | '\n'))
        .filter_map(|clause| {
            prefixed_shellish_command_segment_from_clause(runtime, clause, allow_bare_token)
        })
        .collect()
}

pub(super) fn prefixed_shellish_command_segment_from_clause(
    runtime: &crate::CommandIntentRuntime,
    clause: &str,
    allow_bare_token: bool,
) -> Option<String> {
    let clause = clause.trim();
    if clause.is_empty() {
        return None;
    }
    for (idx, _) in clause.char_indices() {
        let tail = &clause[idx..];
        if let Some(command) =
            prefixed_shellish_command_segment_from_tail(runtime, tail, allow_bare_token)
        {
            return Some(command);
        }
    }
    None
}

pub(super) fn prefixed_shellish_command_segment_from_tail(
    runtime: &crate::CommandIntentRuntime,
    text: &str,
    allow_bare_token: bool,
) -> Option<String> {
    runtime
        .execute_prefixes
        .iter()
        .filter_map(|prefix| strip_configured_command_prefix(text, prefix))
        .filter_map(|tail| {
            let segment = markdown_code_span_command_segment(tail)
                .or_else(|| embedded_markdown_code_span_command_segment(tail))
                .or_else(|| explicit_command_segment_before_followup(tail))
                .or_else(|| whole_explicit_command_tail(tail))?;
            let command = crate::bootstrap::config_loaders::trim_command_text(segment.to_string());
            if literal_command_segment_has_unresolved_template(&command)
                || command.contains('`')
                || !looks_like_concrete_command_tail(&command)
                || (!allow_bare_token
                    && !command
                        .chars()
                        .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<') || ch.is_whitespace()))
            {
                return None;
            }
            Some(command)
        })
        .next()
}

pub(super) fn shellish_literal_command_segment(request: &str) -> Option<String> {
    shellish_literal_command_segments(request, false)
        .into_iter()
        .next()
}

pub(super) fn simple_bare_command_token(token: &str) -> bool {
    !token.is_empty()
        && !token.starts_with('-')
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
        && token
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .count()
            >= 2
}

pub(super) fn command_token_resolves_in_path(
    token: &str,
    path_env: Option<&std::ffi::OsStr>,
) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| dir.join(token).is_file())
}

pub(super) fn leading_shellish_command_sequence_segment_with_path_env(
    request: &str,
    path_env: Option<&std::ffi::OsStr>,
) -> Option<String> {
    let request = request.trim_start();
    if request.is_empty() {
        return None;
    }
    let ascii_end = request
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii()).then_some(idx))
        .unwrap_or(request.len());
    let ascii_prefix = request[..ascii_end].trim();
    if ascii_prefix.is_empty() {
        return None;
    }
    let mut commands = Vec::new();
    for raw_token in ascii_prefix.split_whitespace() {
        let token = raw_token
            .trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
        if !simple_bare_command_token(token) || !command_token_resolves_in_path(token, path_env) {
            break;
        }
        commands.push(token.to_string());
    }
    (commands.len() >= 3).then(|| commands.join("; "))
}

pub(super) fn leading_shellish_command_sequence_segment(request: &str) -> Option<String> {
    let path_env = std::env::var_os("PATH");
    leading_shellish_command_sequence_segment_with_path_env(request, path_env.as_deref())
}

pub(in crate::agent_engine) fn explicit_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    leading_shellish_command_sequence_segment(request)
        .or_else(|| configured_explicit_command_segment(runtime, request))
        .or_else(|| {
            embedded_standalone_command_candidate(runtime, request)
                .map(|candidate| candidate.command)
        })
        .or_else(|| shellish_literal_command_segment(request))
}

pub(in crate::agent_engine) fn explicit_execution_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    leading_shellish_command_sequence_segment(request)
        .or_else(|| configured_explicit_command_segment(runtime, request))
        .or_else(|| {
            embedded_standalone_command_candidate(runtime, request)
                .and_then(|candidate| candidate.single_step_safe.then_some(candidate.command))
        })
}

#[cfg(test)]
pub(super) fn explicit_command_single_step_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
) -> Option<String> {
    if let Some(command) = leading_shellish_command_sequence_segment(request) {
        return Some(command);
    }
    if let Some(candidate) = configured_explicit_command_candidate(runtime, request) {
        return candidate.single_step_safe.then_some(candidate.command);
    }
    if let Some(candidate) = embedded_standalone_command_candidate(runtime, request) {
        return candidate.single_step_safe.then_some(candidate.command);
    }
    shellish_literal_command_segment(request)
        .or_else(|| leading_shellish_command_sequence_segment(request))
}

pub(super) fn route_allows_explicit_command_preservation(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && (route.output_contract.requires_content_evidence
                || route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput))
    })
}

pub(super) fn run_cmd_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("run_cmd")
}

pub(super) fn process_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("process_basic")
}

pub(super) fn system_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("system_basic")
}

pub(super) fn health_check_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("health_check")
}

pub(super) fn action_is_run_cmd(state: &AppState, action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(|skill| state.resolve_canonical_skill_name(skill) == "run_cmd")
        .unwrap_or(false)
}

pub(super) fn literal_command_failure_can_replan(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && !route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
            && !route.output_contract_marker_is(crate::OutputSemanticKind::ExecutionFailedStep)
    })
}

pub(super) fn route_contract_defers_literal_command_to_planner(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && ([
                crate::OutputSemanticKind::StructuredKeys,
                crate::OutputSemanticKind::DirectoryPurposeSummary,
                crate::OutputSemanticKind::DirectoryEntryGroups,
                crate::OutputSemanticKind::FileNames,
                crate::OutputSemanticKind::DirectoryNames,
                crate::OutputSemanticKind::FilePaths,
                crate::OutputSemanticKind::ContentExcerptSummary,
                crate::OutputSemanticKind::ContentExcerptWithSummary,
                crate::OutputSemanticKind::ExistenceWithPath,
                crate::OutputSemanticKind::ExistenceWithPathSummary,
                crate::OutputSemanticKind::RecentScalarEqualityCheck,
                crate::OutputSemanticKind::RecentArtifactsJudgment,
                crate::OutputSemanticKind::SqliteTableListing,
                crate::OutputSemanticKind::SqliteTableNamesOnly,
                crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                crate::OutputSemanticKind::SqliteSchemaVersion,
            ]
            .iter()
            .any(|kind| route.output_contract_marker_is(*kind))
                || (route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly)
                    && scalar_path_contract_has_structural_locator(route)))
    })
}

fn scalar_path_contract_has_structural_locator(route: &RouteResult) -> bool {
    match route.output_contract.locator_kind {
        crate::OutputLocatorKind::CurrentWorkspace => true,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename => {
            !route.output_contract.locator_hint.trim().is_empty()
        }
        _ => false,
    }
}

pub(super) fn missing_target_failure_can_replan(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.is_execute_gate()
            && route.output_contract.requires_content_evidence
            && [
                crate::OutputSemanticKind::FilePaths,
                crate::OutputSemanticKind::FileNames,
                crate::OutputSemanticKind::DirectoryNames,
                crate::OutputSemanticKind::DirectoryPurposeSummary,
                crate::OutputSemanticKind::ContentExcerptSummary,
                crate::OutputSemanticKind::ContentExcerptWithSummary,
                crate::OutputSemanticKind::ExistenceWithPathSummary,
            ]
            .iter()
            .any(|kind| route.output_contract_marker_is(*kind))
    })
}

pub(super) fn mark_missing_target_repairable_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !missing_target_failure_can_replan(route_result) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                let canonical = state.resolve_canonical_skill_name(&skill);
                if matches!(
                    canonical.as_str(),
                    "read_file" | "list_dir" | "system_basic"
                ) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG.to_string(),
                            Value::Bool(true),
                        );
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                let canonical = state.resolve_canonical_skill_name(&tool);
                if matches!(
                    canonical.as_str(),
                    "read_file" | "list_dir" | "system_basic"
                ) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG.to_string(),
                            Value::Bool(true),
                        );
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn mark_explicit_literal_run_cmd_actions(
    actions: Vec<AgentAction>,
    failure_repairable: bool,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if skill.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_LITERAL_COMMAND_ARG.to_string(),
                            Value::Bool(true),
                        );
                        if failure_repairable {
                            obj.insert(
                                super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG.to_string(),
                                Value::Bool(true),
                            );
                        }
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if tool.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_LITERAL_COMMAND_ARG.to_string(),
                            Value::Bool(true),
                        );
                        if failure_repairable {
                            obj.insert(
                                super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG.to_string(),
                                Value::Bool(true),
                            );
                        }
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_single_explicit_literal_run_cmd_command(
    actions: Vec<AgentAction>,
    exact_command: Option<&str>,
) -> Vec<AgentAction> {
    let Some(exact_command) = exact_command
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return actions;
    };
    let run_cmd_count = actions
        .iter()
        .filter(|action| action_skill_is_run_cmd(action))
        .count();
    if run_cmd_count != 1 {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if skill.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        let current = obj.get("command").and_then(Value::as_str).map(str::trim);
                        if current != Some(exact_command) {
                            obj.insert(
                                "command".to_string(),
                                Value::String(exact_command.to_string()),
                            );
                            info!("plan_rewrite_explicit_literal_run_cmd_command");
                        }
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if tool.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        let current = obj.get("command").and_then(Value::as_str).map(str::trim);
                        if current != Some(exact_command) {
                            obj.insert(
                                "command".to_string(),
                                Value::String(exact_command.to_string()),
                            );
                            info!("plan_rewrite_explicit_literal_run_cmd_command");
                        }
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn planned_run_cmds_are_verbatim_user_commands(
    actions: &[AgentAction],
    original_user_text: &str,
) -> bool {
    let mut count = 0usize;
    for action in actions {
        if !action_skill_is_run_cmd(action) {
            continue;
        }
        let Some(command) = run_cmd_command_arg(action) else {
            return false;
        };
        if !request_text_contains_command_verbatim(original_user_text, command) {
            return false;
        }
        count += 1;
    }
    count > 0
}

pub(super) fn replace_explicit_command_substitute_plan_with_run_cmd(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output
        || !route_allows_explicit_command_preservation(route_result)
        || !run_cmd_available_for_plan(state)
    {
        return actions;
    }
    let Some(original_user_text) = original_user_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return actions;
    };
    if planner_has_allowed_capability_ref_action(route_result, &actions) {
        return actions;
    }
    let exact_command = explicit_command_segment(&state.policy.command_intent, original_user_text);
    let has_literal_command_sequence = exact_command.is_some()
        || execution_failed_step_literal_command_segments(
            &state.policy.command_intent,
            original_user_text,
            None,
        )
        .len()
            >= 2;
    let planned_verbatim_run_cmds =
        planned_run_cmds_are_verbatim_user_commands(&actions, original_user_text);
    if !has_literal_command_sequence {
        if !planned_verbatim_run_cmds {
            return actions;
        }
    }
    if actions
        .iter()
        .any(|action| action_is_run_cmd(state, action))
    {
        let actions =
            normalize_single_explicit_literal_run_cmd_command(actions, exact_command.as_deref());
        return mark_explicit_literal_run_cmd_actions(
            actions,
            literal_command_failure_can_replan(route_result),
        );
    }
    if route_contract_defers_literal_command_to_planner(route_result) {
        return actions;
    }
    let Some(first_observation_idx) = actions.iter().position(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) else {
        return actions;
    };
    let Some(exact_command) = exact_command else {
        return actions;
    };
    let mut rewritten = actions;
    let mut args = serde_json::json!({
        "request_text": original_user_text,
        "cwd": state.skill_rt.workspace_root.display().to_string(),
    });
    args["command"] = serde_json::Value::String(exact_command);
    args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    if literal_command_failure_can_replan(route_result) {
        args[super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG] = Value::Bool(true);
    }
    rewritten[first_observation_idx] = AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    };
    info!("plan_rewrite_explicit_command_substitute_to_run_cmd");
    rewritten
}

fn planner_has_allowed_capability_ref_action(
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if crate::machine_capability_ref::route_capability_ref_tokens(route).is_empty() {
        return false;
    }
    actions.iter().any(|action| {
        let Some((skill, args)) = planned_execution_action_ref(action) else {
            return false;
        };
        !skill.eq_ignore_ascii_case("run_cmd")
            && crate::evidence_policy::capability_ref_action_policy_for_route(
                Some(route),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
    })
}

#[cfg(test)]
pub(super) fn normalize_planned_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original(
        state,
        route_result,
        loop_state,
        user_text,
        None,
        auto_locator_path,
        actions,
    )
}

#[cfg(test)]
pub(super) fn normalize_planned_actions_with_original(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        None,
        auto_locator_path,
        actions,
    )
}

fn normalize_action_arg_aliases(state: &AppState, actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallTool { tool, args } => {
                    let normalized = state.resolve_canonical_skill_name(tool);
                    super::super::arg_resolver::normalize_skill_arg_aliases(&normalized, args);
                }
                AgentAction::CallSkill { skill, args } => {
                    let normalized = state.resolve_canonical_skill_name(skill);
                    super::super::arg_resolver::normalize_skill_arg_aliases(&normalized, args);
                }
                AgentAction::CallCapability { .. }
                | AgentAction::SynthesizeAnswer { .. }
                | AgentAction::Respond { .. }
                | AgentAction::Think { .. } => {}
            }
            action
        })
        .collect()
}

pub(super) fn normalize_planned_actions_with_original_and_context(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = crate::capability_resolver::resolve_agent_actions_for_state(state, actions);
    let actions = normalize_action_arg_aliases(state, actions);
    let terminal_mixed_last_output_content = terminal_mixed_last_output_respond_content(&actions);
    let actions = replace_scalar_path_respond_only_with_auto_locator_observation(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = replace_file_delivery_respond_only_with_path_observation(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = replace_file_delivery_empty_write_with_path_observation(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = replace_explicit_command_substitute_plan_with_run_cmd(
        state,
        route_result,
        loop_state,
        original_user_text,
        actions,
    );
    let actions =
        super::super::planning_recent_artifacts::normalize_recent_artifacts_listing_selectors(
            route_result,
            actions,
        );
    let actions =
        super::super::planning_recent_artifacts::rewrite_recent_artifacts_field_extraction_to_selected_file_reads(
            route_result,
            loop_state,
            &state.skill_rt.workspace_root,
            actions,
        );
    let actions = replace_contract_rejected_actions_with_preferred_refs(
        state,
        route_result,
        loop_state,
        original_user_text.or(Some(user_text)),
        auto_locator_path,
        actions,
    );
    let actions =
        ensure_run_cmd_async_start_for_runtime_async_job_contract(state, route_result, actions);
    let actions =
        apply_scalar_count_contract_filter_to_count_entries_actions(route_result, actions);
    let explicit_command_request = route_allows_explicit_command_preservation(route_result)
        && original_user_text.or(Some(user_text)).is_some_and(|text| {
            explicit_command_segment(&state.policy.command_intent, text).is_some()
        });
    let defer_legacy_semantic_rewrites = !explicit_command_request
        && route_result.is_some_and(|route| {
            actions_use_ad_hoc_command_without_route_preferred_skill(state, route, &actions)
        });
    if defer_legacy_semantic_rewrites {
        info!("plan_defer_legacy_semantic_rewrite_to_registry_repair");
    }
    let skip_legacy_semantic_rewrites = explicit_command_request || defer_legacy_semantic_rewrites;
    let actions = normalize_legacy_compatibility_actions(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        plan_context,
        auto_locator_path,
        actions,
        skip_legacy_semantic_rewrites,
    );
    let actions =
        rewrite_process_ps_run_cmd_to_process_basic(state, user_text, original_user_text, actions);
    let actions = rewrite_simple_filesystem_run_cmd_to_fs_basic(
        state,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_append_run_cmd_to_fs_basic(state, user_text, original_user_text, actions);
    let actions = rewrite_readonly_file_read_run_cmd_to_fs_basic(
        state,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_readonly_find_run_cmd_to_fs_basic(
        state,
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions =
        super::super::planning_recent_artifacts::normalize_recent_artifacts_listing_selectors(
            route_result,
            actions,
        );
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_evidence_contract_actions(
        state,
        route_result,
        loop_state,
        original_user_text.unwrap_or(user_text),
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = strip_media_artifact_text_overwrites(&state.skill_rt.workspace_root, actions);
    let actions =
        strip_unrequested_config_edit_actions(route_result, user_text, original_user_text, actions);
    let actions = normalize_terminal_delivery_actions(
        state,
        route_result,
        loop_state,
        user_text,
        terminal_mixed_last_output_content,
        actions,
    );
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let actions = rewrite_single_target_structured_field_read_to_auto_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_active_bound_target_observations_to_matching_locator_hint(
        route_result,
        loop_state,
        actions,
    );
    let actions = rewrite_session_alias_delivery_observations_to_route_locator(
        route_result,
        loop_state,
        actions,
    );
    let actions =
        expand_compound_listing_and_content_synthesis_refs(route_result, loop_state, actions);
    let actions =
        append_terminal_synthesize_for_observation_summary_contract(route_result, actions);
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions = complete_missing_session_alias_target_observations(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        plan_context,
        actions,
    );
    let actions =
        mark_non_mutating_run_cmd_sequences_continue_on_error(state, route_result, actions);
    let actions =
        rewrite_backend_identity_metadata_respond_to_runtime_identity(state, route_result, actions);
    apply_scalar_count_contract_filter_to_count_entries_actions(route_result, actions)
}

fn ensure_run_cmd_async_start_for_runtime_async_job_contract(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !crate::async_job_contract::route_requests_runtime_async_job_contract(route) {
        return actions;
    }
    let mut changed = false;
    let actions = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if state.resolve_canonical_skill_name(&skill) == "run_cmd"
                    && ensure_run_cmd_async_start_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if state.resolve_canonical_skill_name(&tool) == "run_cmd"
                    && ensure_run_cmd_async_start_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_inject_run_cmd_async_start_for_async_job_contract");
    }
    actions
}

fn ensure_run_cmd_async_start_args(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let Some(command) = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return false;
    };
    if run_cmd_command_claims_runtime_async_metadata(command) {
        return false;
    }
    let mut changed = false;
    if obj.get("async_start").and_then(Value::as_bool) != Some(true) {
        obj.insert("async_start".to_string(), Value::Bool(true));
        changed = true;
    }
    if !obj.contains_key("poll_after_seconds") {
        obj.insert("poll_after_seconds".to_string(), Value::from(2));
        changed = true;
    }
    if !obj.contains_key("expires_in_seconds") {
        obj.insert("expires_in_seconds".to_string(), Value::from(600));
        changed = true;
    }
    if obj
        .get(super::super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG)
        .and_then(Value::as_str)
        != Some("async_job_protocol")
    {
        obj.insert(
            super::super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG.to_string(),
            Value::String("async_job_protocol".to_string()),
        );
        changed = true;
    }
    changed
}

fn run_cmd_command_claims_runtime_async_metadata(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    [
        "checkpoint_id",
        "poll_ref",
        "next_check_after",
        "status=background",
        "pending_async_job",
        "job_id=",
    ]
    .iter()
    .any(|token| command.contains(token))
}

fn rewrite_backend_identity_metadata_respond_to_runtime_identity(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_reason_has_backend_identity_metadata_marker(route) {
        return actions;
    }
    let [AgentAction::Respond { content }] = actions.as_slice() else {
        return actions;
    };
    if !respond_content_mentions_backend_identity_metadata(state, content) {
        return actions;
    }
    info!("plan_rewrite_backend_identity_metadata_respond_to_runtime_identity");
    vec![AgentAction::Respond {
        content: state.agent_runtime_identity_label().to_string(),
    }]
}

fn route_reason_has_backend_identity_metadata_marker(route: &RouteResult) -> bool {
    route_reason_has_structural_marker(route, "agent_display_name_hint_backend_metadata_removed")
}

fn respond_content_mentions_backend_identity_metadata(state: &AppState, content: &str) -> bool {
    let normalized_content = normalize_backend_identity_token(content);
    if normalized_content.is_empty() {
        return false;
    }
    state.core.llm_providers.iter().any(|provider| {
        provider
            .config
            .name
            .trim()
            .strip_prefix("vendor-")
            .into_iter()
            .chain([
                provider.config.name.trim(),
                provider.config.model.trim(),
                provider.config.provider_type.trim(),
            ])
            .map(normalize_backend_identity_token)
            .filter(|token| token.len() >= 4)
            .any(|token| normalized_content.contains(&token))
    })
}

fn normalize_backend_identity_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}
