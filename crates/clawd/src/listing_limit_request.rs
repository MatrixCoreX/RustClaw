fn parse_small_zh_number_prefix(text: &str) -> Option<(usize, usize)> {
    let trimmed = text.trim_start();
    let numerals = [
        ("十", 10usize),
        ("九", 9),
        ("八", 8),
        ("七", 7),
        ("六", 6),
        ("五", 5),
        ("四", 4),
        ("三", 3),
        ("二", 2),
        ("两", 2),
        ("一", 1),
    ];
    if let Some(rest) = trimmed.strip_prefix("十") {
        let mut value = 10usize;
        let mut consumed = 1usize;
        for (word, digit) in numerals.iter().skip(1) {
            if rest.starts_with(word) {
                value += *digit;
                consumed += word.chars().count();
                break;
            }
        }
        return Some((value, consumed));
    }
    for (word, digit) in numerals.iter().skip(1) {
        if let Some(rest) = trimmed.strip_prefix(word) {
            let mut value = *digit;
            let mut consumed = word.chars().count();
            if let Some(after_ten) = rest.strip_prefix("十") {
                value *= 10;
                consumed += 1;
                for (tail_word, tail_digit) in numerals.iter().skip(1) {
                    if after_ten.starts_with(tail_word) {
                        value += *tail_digit;
                        consumed += tail_word.chars().count();
                        break;
                    }
                }
            }
            return Some((value, consumed));
        }
    }
    None
}

fn parse_small_en_number_prefix(text: &str) -> Option<(usize, usize)> {
    let trimmed = text.trim_start();
    let words = [
        ("one", 1usize),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
        ("nine", 9),
        ("ten", 10),
    ];
    let lower = trimmed.to_ascii_lowercase();
    for (word, value) in words {
        if !lower.starts_with(word) {
            continue;
        }
        if lower
            .chars()
            .nth(word.len())
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        {
            continue;
        }
        return Some((value, word.chars().count()));
    }
    None
}

fn parse_positive_number_prefix(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    let digit_len = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_len > 0 {
        return trimmed[..digit_len]
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0);
    }
    parse_small_zh_number_prefix(trimmed)
        .map(|(value, _)| value)
        .or_else(|| parse_small_en_number_prefix(trimmed).map(|(value, _)| value))
        .filter(|n| *n > 0)
}

fn trim_listing_limit_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
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
}

fn parse_number_prefix_with_suffix(token: &str) -> Option<(usize, &str)> {
    let trimmed = trim_listing_limit_token(token);
    if trimmed.is_empty() {
        return None;
    }
    let digit_len = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_len > 0 {
        let value = trimmed[..digit_len]
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)?;
        return Some((value, &trimmed[digit_len..]));
    }
    let (value, consumed_chars) = parse_small_zh_number_prefix(trimmed)?;
    let suffix_start = trimmed
        .char_indices()
        .nth(consumed_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());
    Some((value, &trimmed[suffix_start..]))
}

fn parse_number_like_prefix_with_suffix(token: &str) -> Option<(usize, &str)> {
    parse_number_prefix_with_suffix(token).or_else(|| {
        let trimmed = trim_listing_limit_token(token);
        let (value, consumed_chars) = parse_small_en_number_prefix(trimmed)?;
        let suffix_start = trimmed
            .char_indices()
            .nth(consumed_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(trimmed.len());
        Some((value, &trimmed[suffix_start..]))
    })
}

fn token_starts_with_listing_limit_unit(token: &str) -> bool {
    let trimmed = trim_listing_limit_token(token);
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    [
        "个", "条", "项", "行", "份", "files", "file", "entries", "entry", "items", "item",
        "lines", "line", "rows", "row",
    ]
    .iter()
    .any(|needle| lower.starts_with(needle))
}

fn zh_count_unit_requires_listing_subject(token: &str) -> bool {
    let trimmed = trim_listing_limit_token(token).trim_start();
    let Some(rest) = ["个", "条", "项", "行", "份"]
        .iter()
        .find_map(|unit| trimmed.strip_prefix(unit))
    else {
        return false;
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        return false;
    }
    [
        "文件",
        "文件名",
        "目录",
        "子项",
        "条目",
        "项目",
        "日志",
        "记录",
        "entries",
        "entry",
        "items",
        "item",
        "files",
        "file",
        "lines",
        "line",
        "rows",
        "row",
    ]
    .iter()
    .any(|needle| rest.contains(needle))
}

fn token_is_listing_limit_modifier(token: &str) -> bool {
    let lower = trim_listing_limit_token(token).to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "the"
            | "most"
            | "more"
            | "recent"
            | "recently"
            | "modified"
            | "latest"
            | "newest"
            | "updated"
            | "changed"
            | "runtime"
            | "run"
            | "log"
            | "logs"
    )
}

fn suffix_has_listing_limit_unit(suffix: &str) -> bool {
    let trimmed = suffix.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    if token_starts_with_listing_limit_unit(trimmed) {
        if ["个", "条", "项", "行", "份"]
            .iter()
            .any(|unit| trim_listing_limit_token(trimmed).starts_with(unit))
        {
            return zh_count_unit_requires_listing_subject(trimmed);
        }
        return true;
    }
    let tokens = trimmed.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return false;
    }
    let mut modifiers_skipped = 0usize;
    for token in tokens {
        if token_starts_with_listing_limit_unit(token) {
            return true;
        }
        if !token_is_listing_limit_modifier(token) {
            return false;
        }
        modifiers_skipped += 1;
        if modifiers_skipped >= 4 {
            return false;
        }
    }
    false
}

pub(crate) fn requested_listing_limit_from_prompt(prompt: &str) -> Option<usize> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    if crate::intent::surface_signals::extract_field_selector_mentions(trimmed).len() >= 2 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for marker in ["top", "first"] {
        if let Some(idx) = lower.find(marker) {
            let suffix = &trimmed[idx + marker.len()..];
            if let Some(limit) = parse_positive_number_prefix(suffix) {
                return Some(limit);
            }
        }
    }
    for marker in ['前', '头'] {
        if let Some(idx) = trimmed.find(marker) {
            let suffix = &trimmed[idx + marker.len_utf8()..];
            if let Some(limit) = parse_positive_number_prefix(suffix) {
                return Some(limit);
            }
        }
    }
    let starts = std::iter::once(0usize).chain(trimmed.char_indices().skip(1).map(|(idx, _)| idx));
    for idx in starts {
        let prev = if idx == 0 {
            None
        } else {
            trimmed[..idx].chars().next_back()
        };
        if prev.is_some_and(|ch| ch.is_ascii_alphanumeric()) {
            continue;
        }
        let Some((limit, suffix)) = parse_number_like_prefix_with_suffix(&trimmed[idx..]) else {
            continue;
        };
        if suffix_has_listing_limit_unit(suffix) {
            return Some(limit);
        }
    }
    None
}
