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

#[test]
fn stock_name_cleanup_tokens_come_from_config() {
    assert_eq!(normalize_stock_name("贵州茅台股票", &[]), "贵州茅台股票");
    assert_eq!(
        normalize_stock_name("贵州茅台股票", &["股票".to_string()]),
        "贵州茅台"
    );
}

#[test]
fn parse_sina_hq_returns_structured_quote_extra() {
    let body = r#"var hq_str_sh600519="贵州茅台,1500.00,1490.00,1519.80,1525.00,1488.00,0,0,123456,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2026-07-07,15:00:00,00";"#;
    let correction = SymbolCorrection {
        input: "茅台".to_string(),
        matched_name: "贵州茅台".to_string(),
        used_llm: false,
    };

    let (text, extra) = parse_sina_hq(body, "sh600519", Some(&correction)).unwrap();

    assert!(text.contains("message_key=stock.msg.quote"));
    assert!(text.contains("code=SH600519"));
    assert!(text.contains("current=1519.80"));
    assert!(!text.contains("现价"));
    assert_eq!(extra.get("action").and_then(Value::as_str), Some("quote"));
    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("stock.msg.quote")
    );
    assert_eq!(
        extra.get("source_skill").and_then(Value::as_str),
        Some("stock")
    );
    assert_eq!(extra.get("code").and_then(Value::as_str), Some("SH600519"));
    assert_eq!(extra.get("name").and_then(Value::as_str), Some("贵州茅台"));
    assert_eq!(
        extra.get("current").and_then(Value::as_str),
        Some("1519.80")
    );
    assert_eq!(
        extra
            .get("quote")
            .and_then(|quote| quote.get("current"))
            .and_then(Value::as_str),
        Some("1519.80")
    );
    assert_eq!(
        extra
            .get("correction")
            .and_then(|correction| correction.get("reason_code"))
            .and_then(Value::as_str),
        Some("alias_match")
    );
    assert!(extra
        .get("change_pct")
        .and_then(Value::as_f64)
        .is_some_and(|value| value > 1.9 && value < 2.1));
}
