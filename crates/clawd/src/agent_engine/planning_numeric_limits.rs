pub(in crate::agent_engine) fn first_ascii_integer_limit(text: &str) -> Option<u64> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < chars.len() {
        let (start, ch) = chars[idx];
        if !ch.is_ascii_digit() {
            idx += 1;
            continue;
        }

        let mut end_idx = idx + 1;
        while end_idx < chars.len() && chars[end_idx].1.is_ascii_digit() {
            end_idx += 1;
        }
        let end = chars
            .get(end_idx)
            .map(|(pos, _)| *pos)
            .unwrap_or_else(|| text.len());
        let prev = idx.checked_sub(1).map(|prev_idx| chars[prev_idx].1);
        let next = chars.get(end_idx).map(|(_, next)| *next);
        if !digit_run_is_embedded_machine_token(prev, next) {
            if let Some(limit) = parse_limit_token(&text[start..end]) {
                return Some(limit);
            }
        }
        idx = end_idx;
    }
    None
}

fn digit_run_is_embedded_machine_token(prev: Option<char>, next: Option<char>) -> bool {
    prev.is_some_and(is_machine_token_neighbor) || next.is_some_and(is_machine_token_neighbor)
}

fn is_machine_token_neighbor(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '\\')
}

fn parse_limit_token(token: &str) -> Option<u64> {
    token
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.clamp(1, 1000))
}

#[cfg(test)]
#[path = "planning_numeric_limits_tests.rs"]
mod tests;
