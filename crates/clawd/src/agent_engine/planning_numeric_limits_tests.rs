use super::first_ascii_integer_limit;

#[test]
fn parses_standalone_ascii_limits() {
    assert_eq!(first_ascii_integer_limit("list the top 5 files"), Some(5));
    assert_eq!(first_ascii_integer_limit("前 3 个文件"), Some(3));
    assert_eq!(first_ascii_integer_limit("第2个"), Some(2));
}

#[test]
fn ignores_digits_embedded_in_paths_and_identifiers() {
    assert_eq!(
        first_ascii_integer_limit("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"),
        None
    );
    assert_eq!(first_ascii_integer_limit("read package_v2.json"), None);
    assert_eq!(first_ascii_integer_limit("Python 3.11"), None);
}

#[test]
fn clamps_large_standalone_limits() {
    assert_eq!(first_ascii_integer_limit("show 5000 entries"), Some(1000));
}
