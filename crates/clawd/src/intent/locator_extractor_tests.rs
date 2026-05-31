use super::{
    extract_explicit_locator_candidates_for_fallback, extract_explicit_locator_for_fallback,
    structured_locator_tokens, StructuredLocatorTokenKind,
};
use crate::OutputLocatorKind;

#[test]
fn extracts_relative_path_locator_from_mixed_text() {
    let out = extract_explicit_locator_for_fallback(
        "看一下 scripts/nl_tests/fixtures/device_local/configs/app_config.toml，然后用一句大白话说它主要配置了什么",
    )
    .expect("path locator should be extracted");
    assert_eq!(out.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        out.locator_hint,
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
    );
    assert_eq!(out.reason, "explicit_path_locator");
}

#[test]
fn strips_terminal_sentence_period_from_path_locator() {
    let out = extract_explicit_locator_for_fallback(
        "Remember that the note file means scripts/nl_tests/fixtures/device_local/docs/service_notes.md.",
    )
    .expect("path locator should be extracted");

    assert_eq!(out.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        out.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn extracts_url_locator_without_downgrading_to_path() {
    let out = extract_explicit_locator_for_fallback(
        "请求一下 http://127.0.0.1:8787/v1/health ，如果能通就简短总结结果",
    )
    .expect("url locator should be extracted");
    assert_eq!(out.locator_kind, OutputLocatorKind::Url);
    assert_eq!(out.locator_hint, "http://127.0.0.1:8787/v1/health");
    assert_eq!(out.reason, "explicit_url_locator");
}

#[test]
fn ignores_non_locator_tokens() {
    assert!(extract_explicit_locator_for_fallback("给我讲个笑话").is_none());
}

#[test]
fn ignores_python_version_numbers_as_path_locators() {
    assert!(
        extract_explicit_locator_for_fallback("Correction: not Python 3.10, use Python 3.11")
            .is_none()
    );
}

#[test]
fn extracts_filename_locator_from_mixed_delivery_text() {
    let out = extract_explicit_locator_for_fallback("把 README.md 发给我")
        .expect("filename locator should be extracted");
    assert_eq!(out.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(out.locator_hint, "README.md");
    assert_eq!(out.reason, "explicit_filename_locator");
}

#[test]
fn extracts_multiple_explicit_path_locators_from_mixed_text() {
    let out = extract_explicit_locator_candidates_for_fallback(
        "读一下 /tmp/a.md 的开头，然后顺手说 /tmp/b.md 是干什么的",
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].locator_kind, OutputLocatorKind::Path);
    assert_eq!(out[0].locator_hint, "/tmp/a.md");
    assert_eq!(out[1].locator_kind, OutputLocatorKind::Path);
    assert_eq!(out[1].locator_hint, "/tmp/b.md");
}

#[test]
fn structured_locator_tokens_keep_only_structural_locator_shapes() {
    let out = structured_locator_tokens(
        "read docs/report.md and README.md, but not README\nFILE:/tmp/out.txt",
    );
    assert!(out
        .iter()
        .any(|token| token.kind == StructuredLocatorTokenKind::Path
            && token.value == "docs/report.md"));
    assert!(out
        .iter()
        .any(|token| token.kind == StructuredLocatorTokenKind::Filename
            && token.value == "README.md"));
    assert!(out
        .iter()
        .any(|token| token.kind == StructuredLocatorTokenKind::DeliveryToken));
    assert!(!out.iter().any(|token| token.value == "README"));
}
