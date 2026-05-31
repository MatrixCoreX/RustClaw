use super::*;

#[test]
fn parse_llm_alias_response_accepts_schema_valid_json() {
    assert_eq!(
        parse_llm_alias_response(r#"{"alias":"中国移动"}"#).unwrap(),
        "中国移动"
    );
}

#[test]
fn parse_llm_alias_response_rejects_extra_fields_before_falling_back() {
    assert_eq!(
        parse_llm_alias_response(r#"{"alias":"中国移动","reason":"extra"}"#).unwrap(),
        r#"{"alias":"中国移动","reason":"extra"}"#
    );
}

#[test]
fn parse_llm_alias_response_rejects_name_field_json_fallback() {
    assert_eq!(
        parse_llm_alias_response(r#"{"name":"中国移动"}"#).unwrap(),
        r#"{"name":"中国移动"}"#
    );
}
