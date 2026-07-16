fn command_segment_has_unresolved_template(segment: &str) -> bool {
    contains_angle_placeholder_token(segment) || segment_looks_like_output_template(segment)
}

fn contains_angle_placeholder_token(text: &str) -> bool {
    let mut chars = text.char_indices().peekable();
    while let Some((start_idx, ch)) = chars.next() {
        if ch != '<' {
            continue;
        }
        let Some((end_idx, _)) = chars.clone().find(|(_, candidate)| *candidate == '>') else {
            continue;
        };
        let inner = text[start_idx + ch.len_utf8()..end_idx].trim();
        let has_identifier_char = inner.chars().any(|candidate| candidate.is_alphanumeric());
        let placeholder_shaped = !inner.is_empty()
            && inner.chars().all(|candidate| {
                candidate.is_alphanumeric() || matches!(candidate, '_' | '-' | '.' | ' ' | '\t')
            });
        if has_identifier_char && placeholder_shaped {
            return true;
        }
    }
    false
}

fn segment_looks_like_output_template(segment: &str) -> bool {
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

fn concrete_command_segment(segment: &str, allow_bare_token: bool) -> bool {
    !command_segment_has_unresolved_template(segment)
        && segment
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .count()
            >= 2
        && (allow_bare_token
            || segment
                .chars()
                .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<') || ch.is_whitespace()))
}

fn backtick_command_segment(request: &str) -> Option<String> {
    let mut parts = request.split('`');
    parts.next();
    parts
        .step_by(2)
        .map(|segment| crate::bootstrap::config_loaders::trim_command_text(segment.to_string()))
        .find(|segment| concrete_command_segment(segment, true))
}

fn simple_bare_command_token(token: &str) -> bool {
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

fn leading_command_sequence(request: &str, path_env: Option<&std::ffi::OsStr>) -> Option<String> {
    let path_env = path_env?;
    let request = request.trim_start();
    let ascii_end = request
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii()).then_some(idx))
        .unwrap_or(request.len());
    let mut commands = Vec::new();
    for raw_token in request[..ascii_end].trim().split_whitespace() {
        let token = raw_token
            .trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
        let resolves = simple_bare_command_token(token)
            && std::env::split_paths(path_env).any(|dir| dir.join(token).is_file());
        if !resolves {
            break;
        }
        commands.push(token.to_string());
    }
    (commands.len() >= 3).then(|| commands.join("; "))
}

pub(crate) fn explicit_machine_syntax_command_segment(request: &str) -> Option<String> {
    leading_command_sequence(request, std::env::var_os("PATH").as_deref())
        .or_else(|| backtick_command_segment(request))
}

#[cfg(test)]
#[path = "explicit_machine_command_tests.rs"]
mod tests;
