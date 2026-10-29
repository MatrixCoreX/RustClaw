use super::*;

#[test]
fn long_lines_are_skipped_without_losing_following_lines_or_the_full_digest() {
    let text = format!(
        "pub fn before() {{}}\r\n{}\npub fn after() {{}}",
        "x".repeat(4 * 1024 * 1024)
    );
    let result = scan(text.as_bytes()).unwrap();
    assert_eq!(
        result.sha256,
        format!("{:x}", Sha256::digest(text.as_bytes()))
    );
    assert_eq!(result.status, "range_index_budget_exceeded");
    assert_eq!(
        result
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s.line))
            .collect::<Vec<_>>(),
        vec![("before", 1), ("after", 3)]
    );
}

#[test]
fn utf8_crossing_buffer_boundaries_and_exact_line_quota_are_not_truncated() {
    let text = format!(
        "//{}\npub fn retained() {{}}\n",
        "\u{754c}".repeat(MAX_SNIPPET_BYTES / 3 - 1)
    );
    let result = scan(text.as_bytes()).unwrap();
    assert_eq!(result.status, "range_indexed");
    assert_eq!(result.symbols[0].line, 2);
    let exact = format!(
        "{}fn exact() {{}}",
        " ".repeat(MAX_SNIPPET_BYTES - "fn exact() {}".len())
    );
    assert_eq!(scan(exact.as_bytes()).unwrap().symbols[0].name, "exact");
    let beyond = format!(" {exact}\nfn next() {{}}");
    let result = scan(beyond.as_bytes()).unwrap();
    assert_eq!(result.status, "range_index_budget_exceeded");
    assert_eq!(result.symbols.len(), 1);
    assert_eq!(result.symbols[0].name, "next");
}

#[test]
fn invalid_utf8_and_unterminated_long_line_remain_explicit_partial_results() {
    let mut bytes = b"fn valid() {}\n".to_vec();
    bytes.extend([255, 254, b'\n']);
    bytes.extend(std::iter::repeat_n(b'x', MAX_SNIPPET_BYTES + 1));
    let result = scan(bytes.as_slice()).unwrap();
    assert_eq!(result.status, "range_index_budget_exceeded");
    assert_eq!(result.symbols.len(), 1);
    assert_eq!(result.sha256, format!("{:x}", Sha256::digest(&bytes)));
}

#[test]
fn symbol_quota_keeps_full_hashing_and_does_not_grow_the_result_without_bound() {
    let text = "fn item() {}\n".repeat(language_parsers::MAX_ENTRIES + 1);
    let result = scan(text.as_bytes()).unwrap();
    assert_eq!(result.status, "range_index_budget_exceeded");
    assert_eq!(result.symbols.len(), language_parsers::MAX_ENTRIES);
    assert_eq!(
        result.sha256,
        format!("{:x}", Sha256::digest(text.as_bytes()))
    );
}

#[test]
fn long_symbol_names_share_a_bounded_aggregate_byte_budget() {
    let text = format!("fn {}() {{}}\n", "a".repeat(4096)).repeat(300);
    let result = scan(text.as_bytes()).unwrap();
    assert_eq!(result.status, "range_index_budget_exceeded");
    assert!(result.symbols.len() < 300);
    assert!(
        result
            .symbols
            .iter()
            .map(|item| item.name.len() * 2)
            .sum::<usize>()
            <= MAX_SOURCE_BYTES as usize
    );
    assert_eq!(
        result.sha256,
        format!("{:x}", Sha256::digest(text.as_bytes()))
    );
}

#[test]
fn exactly_fitting_symbols_with_trailing_comments_do_not_claim_omitted_definitions() {
    let text = "fn item() {}\n".repeat(language_parsers::MAX_ENTRIES) + "// trailing comment\n";
    let result = scan(text.as_bytes()).unwrap();
    assert_eq!(result.status, "range_indexed");
    assert_eq!(result.symbols.len(), language_parsers::MAX_ENTRIES);
}

#[test]
fn read_failure_never_returns_a_successful_partial_digest() {
    struct FailedReader;
    impl Read for FailedReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture_read_failed"))
        }
    }
    assert!(scan(FailedReader).is_err());
}
