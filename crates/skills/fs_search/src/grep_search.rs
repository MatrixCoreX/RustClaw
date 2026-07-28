use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

pub(super) const MAX_CONTEXT_LINES: usize = 20;
pub(super) const MAX_REGEX_PATTERN_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PatternKind {
    Literal,
    Regex,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct GrepOptions<'a> {
    pub(super) query: &'a str,
    pub(super) pattern_kind: PatternKind,
    pub(super) case_insensitive: bool,
    pub(super) multiline: bool,
    pub(super) context_before: usize,
    pub(super) context_after: usize,
    pub(super) max_line_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ContextLine {
    line: usize,
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct GrepMatch {
    pub(super) line: usize,
    pub(super) end_line: usize,
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) text: String,
    pub(super) matched_text: String,
    pub(super) line_start_byte: usize,
    pub(super) line_end_byte: usize,
    pub(super) context_before: Vec<ContextLine>,
    pub(super) context_after: Vec<ContextLine>,
}

#[derive(Debug)]
struct LineView<'a> {
    number: usize,
    start_byte: usize,
    end_byte: usize,
    text: &'a str,
}

pub(super) fn find_matches(text: &str, options: GrepOptions<'_>) -> Result<Vec<GrepMatch>, String> {
    if options.query.is_empty() {
        return Err("query_empty".to_string());
    }
    if options.pattern_kind == PatternKind::Regex && options.query.len() > MAX_REGEX_PATTERN_BYTES {
        return Err("regex_pattern_too_large".to_string());
    }
    let lines = line_views(text);
    let matcher = build_matcher(options)?;
    if options.multiline {
        Ok(find_multiline_matches(text, &lines, options, &matcher))
    } else {
        Ok(find_line_matches(&lines, options, &matcher))
    }
}

fn build_matcher(options: GrepOptions<'_>) -> Result<Regex, String> {
    let pattern = match options.pattern_kind {
        PatternKind::Literal => regex::escape(options.query),
        PatternKind::Regex => options.query.to_string(),
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(options.case_insensitive)
        .multi_line(options.multiline)
        .dot_matches_new_line(options.multiline)
        .size_limit(4 * 1024 * 1024)
        .dfa_size_limit(8 * 1024 * 1024)
        .build()
        .map_err(|_| "regex_pattern_invalid".to_string())
}

fn find_line_matches(
    lines: &[LineView<'_>],
    options: GrepOptions<'_>,
    matcher: &Regex,
) -> Vec<GrepMatch> {
    lines
        .iter()
        .enumerate()
        .flat_map(|(index, line)| {
            matcher
                .find_iter(line.text)
                .filter(|matched| matched.start() != matched.end())
                .map(move |matched| GrepMatch {
                    line: line.number,
                    end_line: line.number,
                    start_byte: line.start_byte.saturating_add(matched.start()),
                    end_byte: line.start_byte.saturating_add(matched.end()),
                    text: truncate_chars(line.text.trim(), options.max_line_chars),
                    matched_text: truncate_chars(matched.as_str(), options.max_line_chars),
                    line_start_byte: line.start_byte,
                    line_end_byte: line.end_byte,
                    context_before: context_before(lines, index, options),
                    context_after: context_after(lines, index, options),
                })
        })
        .collect()
}

fn find_multiline_matches(
    text: &str,
    lines: &[LineView<'_>],
    options: GrepOptions<'_>,
    matcher: &Regex,
) -> Vec<GrepMatch> {
    matcher
        .find_iter(text)
        .filter(|matched| matched.start() != matched.end())
        .map(|matched| {
            let start_index = line_index_for_byte(lines, matched.start());
            let end_index =
                line_index_for_byte(lines, matched.end().saturating_sub(1)).max(start_index);
            GrepMatch {
                line: lines.get(start_index).map(|line| line.number).unwrap_or(1),
                end_line: lines.get(end_index).map(|line| line.number).unwrap_or(1),
                start_byte: matched.start(),
                end_byte: matched.end(),
                text: truncate_chars(matched.as_str(), options.max_line_chars),
                matched_text: truncate_chars(matched.as_str(), options.max_line_chars),
                line_start_byte: lines
                    .get(start_index)
                    .map(|line| line.start_byte)
                    .unwrap_or(0),
                line_end_byte: lines.get(end_index).map(|line| line.end_byte).unwrap_or(0),
                context_before: context_before(lines, start_index, options),
                context_after: context_after(lines, end_index, options),
            }
        })
        .collect()
}

fn line_views(text: &str) -> Vec<LineView<'_>> {
    if text.is_empty() {
        return vec![LineView {
            number: 1,
            start_byte: 0,
            end_byte: 0,
            text: "",
        }];
    }
    let mut lines = Vec::new();
    let mut start_byte = 0;
    for (index, segment) in text.split_inclusive('\n').enumerate() {
        let without_lf = segment.strip_suffix('\n').unwrap_or(segment);
        let line = without_lf.strip_suffix('\r').unwrap_or(without_lf);
        lines.push(LineView {
            number: index + 1,
            start_byte,
            end_byte: start_byte + line.len(),
            text: line,
        });
        start_byte += segment.len();
    }
    if text.ends_with('\n') {
        lines.push(LineView {
            number: lines.len() + 1,
            start_byte,
            end_byte: start_byte,
            text: "",
        });
    }
    lines
}

fn line_index_for_byte(lines: &[LineView<'_>], byte: usize) -> usize {
    lines
        .partition_point(|line| line.start_byte <= byte)
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1))
}

fn context_before(
    lines: &[LineView<'_>],
    index: usize,
    options: GrepOptions<'_>,
) -> Vec<ContextLine> {
    let start = index.saturating_sub(options.context_before);
    lines[start..index]
        .iter()
        .map(|line| context_line(line, options.max_line_chars))
        .collect()
}

fn context_after(
    lines: &[LineView<'_>],
    index: usize,
    options: GrepOptions<'_>,
) -> Vec<ContextLine> {
    let start = index.saturating_add(1).min(lines.len());
    let end = start.saturating_add(options.context_after).min(lines.len());
    lines[start..end]
        .iter()
        .map(|line| context_line(line, options.max_line_chars))
        .collect()
}

fn context_line(line: &LineView<'_>, max_chars: usize) -> ContextLine {
    ContextLine {
        line: line.number,
        text: truncate_chars(line.text, max_chars),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
#[path = "grep_search_tests.rs"]
mod tests;
