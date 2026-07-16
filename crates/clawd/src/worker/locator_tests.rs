#[test]
fn detects_language_independent_locator_syntax() {
    for value in [
        "/workspace/README.md",
        "scripts/check.py",
        "./document",
        "Cargo.toml",
        "https://example.com/file.txt",
    ] {
        assert!(super::has_concrete_locator_hint(value), "value={value}");
    }
}

#[test]
fn rejects_versions_and_protocol_field_selectors() {
    for value in ["3.11", "text/error_text", "RepairEnvelope/issue_codes"] {
        assert!(!super::has_concrete_locator_hint(value), "value={value}");
    }
}

#[test]
fn explicit_locator_check_requires_path_or_url_syntax() {
    assert!(super::has_explicit_path_or_url_locator_hint(
        "../src/main.rs"
    ));
    assert!(super::has_explicit_path_or_url_locator_hint(
        "https://example.com/data.json"
    ));
    assert!(!super::has_explicit_path_or_url_locator_hint("Cargo.toml"));
}
