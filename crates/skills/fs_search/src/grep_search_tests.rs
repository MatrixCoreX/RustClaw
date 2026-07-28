use super::*;

fn options<'a>(query: &'a str) -> GrepOptions<'a> {
    GrepOptions {
        query,
        pattern_kind: PatternKind::Literal,
        case_insensitive: false,
        multiline: false,
        context_before: 0,
        context_after: 0,
        max_line_chars: 240,
    }
}

#[test]
fn line_matches_include_bounded_context_and_byte_range() {
    let text = "alpha\nbefore\nneedle value\nafter\nomega\n";
    let mut options = options("needle");
    options.context_before = 1;
    options.context_after = 1;

    let matches = find_matches(text, options).expect("line search");

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line, 3);
    assert_eq!(matches[0].end_line, 3);
    assert_eq!(&text[matches[0].start_byte..matches[0].end_byte], "needle");
    assert_eq!(matches[0].context_before[0].line, 2);
    assert_eq!(matches[0].context_before[0].text, "before");
    assert_eq!(matches[0].context_after[0].line, 4);
    assert_eq!(matches[0].context_after[0].text, "after");
}

#[test]
fn multiline_literal_and_wildcard_report_exact_provenance() {
    let text = "fn demo() {\n    let value = 1;\n    finish(value);\n}\n";
    let mut literal = options("let value = 1;\n    finish(value)");
    literal.multiline = true;
    let literal_matches = find_matches(text, literal).expect("literal multiline");
    assert_eq!(literal_matches.len(), 1);
    assert_eq!(literal_matches[0].line, 2);
    assert_eq!(literal_matches[0].end_line, 3);
    assert_eq!(
        &text[literal_matches[0].start_byte..literal_matches[0].end_byte],
        "let value = 1;\n    finish(value)"
    );

    let mut wildcard = options("let value.*finish");
    wildcard.pattern_kind = PatternKind::Regex;
    wildcard.multiline = true;
    let wildcard_matches = find_matches(text, wildcard).expect("wildcard multiline");
    assert_eq!(wildcard_matches.len(), 1);
    assert_eq!(wildcard_matches[0].line, 2);
    assert_eq!(wildcard_matches[0].end_line, 3);
}

#[test]
fn multiline_case_insensitive_keeps_utf8_byte_offsets() {
    let text = "BEGIN\n中文内容\nEND\n";
    let mut options = options("begin\n中文内容\nend");
    options.multiline = true;
    options.case_insensitive = true;

    let matches = find_matches(text, options).expect("case-insensitive multiline");

    assert_eq!(matches.len(), 1);
    assert_eq!(
        &text[matches[0].start_byte..matches[0].end_byte],
        "BEGIN\n中文内容\nEND"
    );
}

#[test]
fn context_never_exceeds_requested_boundaries() {
    let text = "zero\none\ntwo\nthree\nfour\n";
    let mut options = options("two");
    options.context_before = MAX_CONTEXT_LINES;
    options.context_after = MAX_CONTEXT_LINES;

    let matches = find_matches(text, options).expect("bounded context");

    assert_eq!(matches[0].context_before.len(), 2);
    assert_eq!(matches[0].context_after.len(), 3);
}

#[test]
fn empty_query_is_rejected() {
    let error = find_matches("content", options("")).expect_err("empty query");
    assert_eq!(error, "query_empty");
}
