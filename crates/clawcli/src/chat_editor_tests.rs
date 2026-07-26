use super::{has_open_code_fence, has_unescaped_trailing_backslash, normalize_multiline_input};

#[test]
fn multiline_validation_uses_explicit_terminal_syntax() {
    assert!(has_unescaped_trailing_backslash("first \\"));
    assert!(!has_unescaped_trailing_backslash("escaped \\\\"));
    assert!(has_open_code_fence("```\ncode"));
    assert!(!has_open_code_fence("```\ncode\n```"));
    assert_eq!(
        normalize_multiline_input("first \\\nsecond"),
        "first second"
    );
}
