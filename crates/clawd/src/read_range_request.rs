use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestedReadRange {
    Head { n: u64 },
    Tail { n: u64 },
    Range { start_line: u64, end_line: u64 },
}

fn parse_range_capture(captures: &regex::Captures<'_>, index: usize) -> Option<u64> {
    captures.get(index)?.as_str().trim().parse::<u64>().ok()
}

pub(crate) fn extract_explicit_read_range_request(text: &str) -> Option<RequestedReadRange> {
    static ZH_RANGE_RE: OnceLock<Regex> = OnceLock::new();
    static EN_RANGE_RE: OnceLock<Regex> = OnceLock::new();
    static ZH_TAIL_RE: OnceLock<Regex> = OnceLock::new();
    static EN_TAIL_RE: OnceLock<Regex> = OnceLock::new();
    static ZH_HEAD_RE: OnceLock<Regex> = OnceLock::new();
    static EN_HEAD_RE: OnceLock<Regex> = OnceLock::new();

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let zh_range_re = ZH_RANGE_RE.get_or_init(|| {
        Regex::new(r"第\s*(\d{1,5})\s*(?:到|至|-|~|—|–)\s*(\d{1,5})\s*行")
            .expect("zh line range regex")
    });
    if let Some(captures) = zh_range_re.captures(trimmed) {
        let start_line = parse_range_capture(&captures, 1)?;
        let end_line = parse_range_capture(&captures, 2)?;
        if start_line > 0 && end_line >= start_line {
            return Some(RequestedReadRange::Range {
                start_line,
                end_line,
            });
        }
    }

    let en_range_re = EN_RANGE_RE.get_or_init(|| {
        Regex::new(r"(?i)\blines?\s+(\d{1,5})\s*(?:to|through|-)\s*(\d{1,5})\b")
            .expect("en line range regex")
    });
    if let Some(captures) = en_range_re.captures(trimmed) {
        let start_line = parse_range_capture(&captures, 1)?;
        let end_line = parse_range_capture(&captures, 2)?;
        if start_line > 0 && end_line >= start_line {
            return Some(RequestedReadRange::Range {
                start_line,
                end_line,
            });
        }
    }

    let zh_tail_re = ZH_TAIL_RE.get_or_init(|| {
        Regex::new(r"(?:最后|最近)\s*(?:读|看)?\s*(\d{1,4})\s*行").expect("zh tail regex")
    });
    if let Some(captures) = zh_tail_re.captures(trimmed) {
        let n = parse_range_capture(&captures, 1)?.clamp(1, 200);
        return Some(RequestedReadRange::Tail { n });
    }

    let en_tail_re = EN_TAIL_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:last|tail)\s+(\d{1,4})\s+lines?\b").expect("en tail regex")
    });
    if let Some(captures) = en_tail_re.captures(trimmed) {
        let n = parse_range_capture(&captures, 1)?.clamp(1, 200);
        return Some(RequestedReadRange::Tail { n });
    }

    let zh_head_re = ZH_HEAD_RE.get_or_init(|| {
        Regex::new(r"(?:前|开头(?:的)?|最前面)\s*(?:读|看)?\s*(\d{1,4})\s*行")
            .expect("zh head regex")
    });
    if let Some(captures) = zh_head_re.captures(trimmed) {
        let n = parse_range_capture(&captures, 1)?.clamp(1, 200);
        return Some(RequestedReadRange::Head { n });
    }

    let en_head_re = EN_HEAD_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:first|head)\s+(\d{1,4})\s+lines?\b").expect("en head regex")
    });
    if let Some(captures) = en_head_re.captures(trimmed) {
        let n = parse_range_capture(&captures, 1)?.clamp(1, 200);
        return Some(RequestedReadRange::Head { n });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{extract_explicit_read_range_request, RequestedReadRange};

    #[test]
    fn extracts_tail_request_in_zh() {
        assert_eq!(
            extract_explicit_read_range_request("看看最后 5 行"),
            Some(RequestedReadRange::Tail { n: 5 })
        );
    }

    #[test]
    fn extracts_recent_tail_request_in_zh_with_verb() {
        assert_eq!(
            extract_explicit_read_range_request("看一下那个日志最近 20 行"),
            Some(RequestedReadRange::Tail { n: 20 })
        );
    }

    #[test]
    fn extracts_head_request_in_zh_with_verb() {
        assert_eq!(
            extract_explicit_read_range_request("把 README.md 开头读 10 行"),
            Some(RequestedReadRange::Head { n: 10 })
        );
    }

    #[test]
    fn extracts_range_request_in_en() {
        assert_eq!(
            extract_explicit_read_range_request("show lines 12 to 18"),
            Some(RequestedReadRange::Range {
                start_line: 12,
                end_line: 18,
            })
        );
    }
}
