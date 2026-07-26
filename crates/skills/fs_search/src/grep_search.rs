use regex::RegexBuilder;
use serde::Serialize;

pub(super) const MAX_CONTEXT_LINES: usize = 20;

#[derive(Debug, Clone, Copy)]
pub(super) struct GrepOptions<'a> {
    pub(super) query: &'a str,
    pub(super) case_insensitive: bool,
    pub(super) multiline: bool,
    pub(super) context_before: usize,
    pub(super) context_after: usize,
    pub(super) max_line_chars: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ContextLine {
    line: usize,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct GrepMatch {
    pub(super) line: usize,
    pub(super) end_line: usize,
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) text: String,
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
    let lines = line_views(text);
    if options.multiline {
        find_multiline_matches(text, &lines, options)
    } else {
        Ok(find_line_matches(&lines, options))
    }
}

fn find_line_matches(lines: &[LineView<'_>], options: GrepOptions<'_>) -> Vec<GrepMatch> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line_matches_query(line.text, options))
        .map(|(index, line)| GrepMatch {
            line: line.number,
            end_line: line.number,
            start_byte: line.start_byte,
            end_byte: line.end_byte,
            text: truncate_chars(line.text.trim(), options.max_line_chars),
            context_before: context_before(lines, index, options),
            context_after: context_after(lines, index, options),
        })
        .collect()
}

fn find_multiline_matches(
    text: &str,
    lines: &[LineView<'_>],
    options: GrepOptions<'_>,
) -> Result<Vec<GrepMatch>, String> {
    let pattern = multiline_pattern(options.query);
    let matcher = RegexBuilder::new(&pattern)
        .case_insensitive(options.case_insensitive)
        .multi_line(true)
        .dot_matches_new_line(true)
        .build()
        .map_err(|_| "multiline_query_invalid".to_string())?;

    Ok(matcher
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
                context_before: context_before(lines, start_index, options),
                context_after: context_after(lines, end_index, options),
            }
        })
        .collect())
}

fn multiline_pattern(query: &str) -> String {
    if !query.contains(".*") {
        return regex::escape(query);
    }
    query
        .split(".*")
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join("(?s:.*?)")
}

fn line_matches_query(line: &str, options: GrepOptions<'_>) -> bool {
    if options.case_insensitive {
        let line_folded = line.to_lowercase();
        let query_folded = options.query.to_lowercase();
        return line_folded.contains(&query_folded)
            || ordered_wildcard_query_matches(&line_folded, &query_folded);
    }
    line.contains(options.query) || ordered_wildcard_query_matches(line, options.query)
}

fn ordered_wildcard_query_matches(line: &str, query: &str) -> bool {
    if !query.contains(".*") {
        return false;
    }
    let parts = query
        .split(".*")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return false;
    }
    let mut rest = line;
    for part in parts {
        let Some(index) = rest.find(part) else {
            return false;
        };
        rest = &rest[index + part.len()..];
    }
    true
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
