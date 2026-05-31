use super::*;

#[test]
fn extracts_stock_anchor_from_result_text() {
    let anchor = extract_execution_anchor(
        "ask",
        r#"{"text":"查询中芯国际今天涨跌情况"}"#,
        r#"{"text":"subtask#1 skill(stock): success [SH688981] 中芯国际 现价106.020 今开108.540 昨收108.600"}"#,
        "1710668477",
    )
    .expect("anchor");
    assert_eq!(anchor.skill, "stock");
    assert_eq!(anchor.domain, "cn_stock");
    assert_eq!(anchor.symbol.as_deref(), Some("688981"));
    assert_eq!(anchor.subject.as_deref(), Some("中芯国际"));
}

#[test]
fn extracts_crypto_anchor_from_result_text() {
    let anchor = extract_execution_anchor(
        "ask",
        r#"{"text":"分析下行情"}"#,
        r#"{"text":"subtask#1 skill(crypto): success BTCUSDT RSI(14)=54.2"}"#,
        "1710668477",
    )
    .expect("anchor");
    assert_eq!(anchor.skill, "crypto");
    assert_eq!(anchor.domain, "crypto");
    assert_eq!(anchor.symbol.as_deref(), Some("BTCUSDT"));
}
