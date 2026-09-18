//! Central compatibility decoder for legacy channel delivery-token lines.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegacyDeliveryKind {
    Image,
    Video,
    Voice,
    Music,
    File,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegacyDeliveryLocation {
    LocalFile,
    RemoteUrl,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LegacyDeliveryToken {
    pub kind: LegacyDeliveryKind,
    pub location: LegacyDeliveryLocation,
    pub reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeliveryTokenRef<'a> {
    pub kind: LegacyDeliveryKind,
    pub location: LegacyDeliveryLocation,
    pub reference: &'a str,
}

const LEGACY_PREFIXES: &[(&str, LegacyDeliveryKind, LegacyDeliveryLocation)] = &[
    (
        "IMAGE_FILE:",
        LegacyDeliveryKind::Image,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "VIDEO_FILE:",
        LegacyDeliveryKind::Video,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "VOICE_FILE:",
        LegacyDeliveryKind::Voice,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "MUSIC_FILE:",
        LegacyDeliveryKind::Music,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "AUDIO_FILE:",
        LegacyDeliveryKind::Music,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "FILE_FILE:",
        LegacyDeliveryKind::File,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "FILE:",
        LegacyDeliveryKind::Auto,
        LegacyDeliveryLocation::LocalFile,
    ),
    (
        "IMAGE_URL:",
        LegacyDeliveryKind::Image,
        LegacyDeliveryLocation::RemoteUrl,
    ),
    (
        "VIDEO_URL:",
        LegacyDeliveryKind::Video,
        LegacyDeliveryLocation::RemoteUrl,
    ),
    (
        "FILE_URL:",
        LegacyDeliveryKind::File,
        LegacyDeliveryLocation::RemoteUrl,
    ),
    (
        "MEDIA_URL:",
        LegacyDeliveryKind::Auto,
        LegacyDeliveryLocation::RemoteUrl,
    ),
];

pub fn parse_legacy_delivery_line_ref(line: &str) -> Option<LegacyDeliveryTokenRef<'_>> {
    let line = strip_delivery_list_prefix(line.trim());
    for (prefix, kind, location) in LEGACY_PREFIXES {
        let Some(raw_reference) = line.strip_prefix(prefix) else {
            continue;
        };
        return Some(LegacyDeliveryTokenRef {
            kind: *kind,
            location: *location,
            reference: raw_reference,
        });
    }
    None
}

/// Accept presentation-only list markers that a finalizer may add around an otherwise canonical
/// delivery-token line. Requiring whitespace after the marker keeps inline prose and filenames
/// such as `1.IMAGE_FILE:example` outside the protocol boundary.
fn strip_delivery_list_prefix(line: &str) -> &str {
    for prefix in ["- ", "* ", "+ ", "• "] {
        if let Some(remainder) = line.strip_prefix(prefix) {
            return remainder.trim_start();
        }
    }

    let digit_count = line
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return line;
    }

    let Some(marker) = line.as_bytes().get(digit_count) else {
        return line;
    };
    if !matches!(marker, b'.' | b')') {
        return line;
    }

    let remainder = &line[digit_count + 1..];
    if remainder.chars().next().is_some_and(char::is_whitespace) {
        remainder.trim_start()
    } else {
        line
    }
}

pub fn parse_legacy_delivery_line(line: &str) -> Option<LegacyDeliveryToken> {
    let token = parse_legacy_delivery_line_ref(line)?;
    let reference = normalize_legacy_delivery_reference(token.reference);
    if reference.is_empty() {
        return None;
    }
    Some(LegacyDeliveryToken {
        kind: token.kind,
        location: token.location,
        reference,
    })
}

pub fn legacy_delivery_tokens(text: &str) -> Vec<LegacyDeliveryToken> {
    text.lines()
        .filter_map(parse_legacy_delivery_line)
        .collect()
}

pub fn strip_legacy_delivery_lines(text: &str) -> String {
    strip_matching_legacy_lines(text, |_| true)
}

pub fn strip_legacy_local_delivery_lines(text: &str) -> String {
    strip_matching_legacy_lines(text, |token| {
        token.location == LegacyDeliveryLocation::LocalFile
    })
}

pub fn legacy_delivery_lines(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| parse_legacy_delivery_line_ref(line).is_some())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn legacy_local_delivery_lines(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            parse_legacy_delivery_line_ref(line)
                .is_some_and(|token| token.location == LegacyDeliveryLocation::LocalFile)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn normalize_legacy_delivery_reference(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';' | '。' | ')' | '(' | '）' | '（'
            )
        })
        .to_string()
}

const TASK_ARTIFACT_HANDLE_PREFIX: &str = "artifact:task/";

/// Remove leftover `PREFIX:artifact:task/...` (and bare handles) from caption
/// prose. Whole token lines stay a separate protocol concern.
pub fn scrub_inline_task_artifact_handles(text: &str) -> String {
    text.lines()
        .map(scrub_inline_task_artifact_handles_in_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn scrub_inline_task_artifact_handles_in_line(line: &str) -> String {
    let mut rest = line;
    let mut out = String::new();
    while let Some((start, end)) = next_inline_task_artifact_span(rest) {
        out.push_str(&rest[..start]);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.trim_end_matches(|ch: char| matches!(ch, '：' | ':' | ' ' | '\t'))
        .to_string()
}

fn next_inline_task_artifact_span(text: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for (prefix, _, _) in LEGACY_PREFIXES {
        let needle = format!("{prefix}{TASK_ARTIFACT_HANDLE_PREFIX}");
        if let Some(idx) = text.find(&needle) {
            let end = handle_span_end(text, idx + prefix.len());
            if best.is_none_or(|(start, _)| idx < start) {
                best = Some((idx, end));
            }
        }
    }
    if let Some(idx) = text.find(TASK_ARTIFACT_HANDLE_PREFIX) {
        if best.is_none_or(|(start, _)| idx < start) {
            best = Some((idx, handle_span_end(text, idx)));
        }
    }
    best
}

fn handle_span_end(text: &str, handle_start: usize) -> usize {
    text[handle_start..]
        .find(is_task_artifact_handle_terminator)
        .map(|offset| handle_start + offset)
        .unwrap_or(text.len())
}

fn is_task_artifact_handle_terminator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '，' | ',' | ';' | '。' | ')' | '）')
}

fn strip_matching_legacy_lines(
    text: &str,
    predicate: impl Fn(&LegacyDeliveryTokenRef<'_>) -> bool,
) -> String {
    text.lines()
        .filter(|line| {
            parse_legacy_delivery_line_ref(line)
                .as_ref()
                .is_none_or(|token| !predicate(token))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "channel_delivery_tokens_tests.rs"]
mod tests;
