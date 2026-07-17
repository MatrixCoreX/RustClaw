use super::plan_json_has_unterminated_string;

#[test]
fn unterminated_terminal_response_string_is_rejected() {
    let raw = r#"{"steps":[{"type":"respond","content":"complete prefix then trunc"#;

    assert!(plan_json_has_unterminated_string(raw));
}

#[test]
fn complete_plan_with_escaped_quotes_is_not_rejected() {
    let raw = r#"{"steps":[{"type":"respond","content":"a \"quoted\" complete answer"}]}"#;

    assert!(!plan_json_has_unterminated_string(raw));
}

#[test]
fn non_json_tool_protocol_is_left_for_its_own_parser() {
    let raw = r#"<invoke name="call_tool"><parameter name="tool">demo</parameter></invoke>"#;

    assert!(!plan_json_has_unterminated_string(raw));
}
