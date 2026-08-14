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
        reason_code: "configured_alias",
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
    assert_eq!(
        extra.get("normalized_code").and_then(Value::as_str),
        Some("SH600519")
    );
    assert_eq!(extra.get("name").and_then(Value::as_str), Some("贵州茅台"));
    assert_eq!(extra.get("price").and_then(Value::as_str), Some("1519.80"));
    assert_eq!(
        extra.get("provider").and_then(Value::as_str),
        Some("sina_finance")
    );
    assert_eq!(
        extra.get("observed_at").and_then(Value::as_str),
        Some("2026-07-07T15:00:00+08:00")
    );
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
        Some("configured_alias")
    );
    assert!(extra
        .get("change_pct")
        .and_then(Value::as_f64)
        .is_some_and(|value| value > 1.9 && value < 2.1));
}

#[test]
fn preview_quote_normalizes_code_without_external_calls() {
    let args = json!({"action": "preview_quote", "code": "600519"});
    let (text, extra) = execute(args, &RuntimeConfig::default()).unwrap();

    assert!(text.contains("message_key=skill.stock.quote_preview_ready"));
    assert_eq!(extra["action"], "preview_quote");
    assert_eq!(extra["normalized_code"], "SH600519");
    assert_eq!(extra["resolution_mode"], "direct_code");
    assert_eq!(extra["would_execute"], false);
    assert_eq!(extra["external_call_count"], 0);
}

#[test]
fn preview_quote_keeps_name_resolution_deferred() {
    let args = json!({"action": "preview_quote", "name": "Example Holdings"});
    let (_, extra) = execute(args, &RuntimeConfig::default()).unwrap();

    assert!(extra["normalized_code"].is_null());
    assert_eq!(
        extra["resolution_mode"],
        "provider_search_or_configured_alias"
    );
    assert_eq!(extra["external_call_count"], 0);
}

#[test]
fn preview_quote_normalizes_us_ticker_without_external_calls() {
    let args = json!({"action": "preview_quote", "symbol": "TSLA"});
    let (_, extra) = execute(args, &RuntimeConfig::default()).unwrap();

    assert_eq!(extra["normalized_code"], "US:TSLA");
    assert_eq!(extra["market"], "us");
    assert_eq!(extra["resolution_mode"], "direct_code");
    assert_eq!(extra["external_call_count"], 0);
}

#[test]
fn sina_symbol_search_resolves_unconfigured_a_share_and_us_names() {
    let body = r#"var suggestdata="园林股份,11,605303,sh605303,园林股份,,园林股份,99,1,,,;特斯拉,41,tsla,tsla,特斯拉,,特斯拉,99,1,ESG,,;";"#;
    let candidates = parse_sina_suggestions(body, &[]);

    let garden = choose_search_candidate("园林股份", "园林股份", &candidates).unwrap();
    assert_eq!(garden.market, StockMarket::China);
    assert_eq!(garden.code, "sh605303");

    let tesla = choose_search_candidate("特斯拉", "特斯拉", &candidates).unwrap();
    assert_eq!(tesla.market, StockMarket::UnitedStates);
    assert_eq!(tesla.code, "TSLA");
}

#[test]
fn parse_sina_us_quote_returns_normalized_market_fields() {
    let body = r#"var hq_str_gb_tsla="特斯拉,349.9528,2.94,2026-08-14 22:00:53,9.9928,342.3300,350.3500,342.0100,498.8300,297.3800,10009980,31333314,1382155169263,1.20,291.490000,0.00,0.00,0.00,0.00,3949547394,61,0.0000,0.00,0.00,,Aug 14 10:00AM EDT,339.9600";"#;
    let (_, extra) = parse_sina_us_hq(body, "TSLA", None).unwrap();

    assert_eq!(extra["normalized_code"], "US:TSLA");
    assert_eq!(extra["market"], "us");
    assert_eq!(extra["currency"], "USD");
    assert_eq!(extra["price"], "349.9528");
    assert_eq!(extra["prev_close"], "339.9600");
}

#[test]
fn parse_tencent_quote_supports_a_share_fallback() {
    let body = r#"v_sh605303="1~园林股份~605303~21.42~21.75~22.00~30682~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~0~20260814161456~-0.33~-1.52~22.07~21.00~CNY";"#;
    let (_, extra) = parse_tencent_hq(body, StockMarket::China, "sh605303", None).unwrap();

    assert_eq!(extra["normalized_code"], "SH605303");
    assert_eq!(extra["provider"], "tencent_finance");
    assert_eq!(extra["observed_at"], "2026-08-14T16:14:56+08:00");
}

#[test]
fn protocol_errors_always_expose_stable_machine_fields() {
    let extra = stock_error_extra("unsupported_action");
    assert_eq!(extra["error_code"], "unsupported_action");
    assert_eq!(extra["message_key"], "skill.stock.unsupported_action");
}

#[test]
fn machine_error_code_preserves_specific_failure_ownership() {
    assert_eq!(
        machine_error_code("code=symbol_not_found input=unknown"),
        "symbol_not_found"
    );
    assert_eq!(
        machine_error_code("provider returned arbitrary prose"),
        "stock_execution_failed"
    );
    assert!(error_is_retryable("quote_provider_chain_failed"));
    assert!(!error_is_retryable("symbol_not_found"));
}

#[test]
fn requested_symbol_is_projected_without_parsing_error_text() {
    assert_eq!(
        requested_symbol_from_args(&json!({"code": " NOT-A-STOCK "})),
        Some("NOT-A-STOCK")
    );
}
