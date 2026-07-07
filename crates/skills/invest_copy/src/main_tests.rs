use super::*;

#[test]
fn error_extra_merges_machine_contract_and_details() {
    let extra = error_extra_with_details(
        "data_too_short",
        Some(json!({
            "current_chars": 3,
            "min_chars": MIN_DATA_CHARS
        })),
    );

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "data_too_short");
    assert_eq!(extra["message_key"], "skill.invest_copy.data_too_short");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["current_chars"], 3);
    assert_eq!(extra["min_chars"], MIN_DATA_CHARS);
}

#[test]
fn draft_missing_person_returns_machine_error_extra() {
    let lookup = std::collections::HashMap::new();
    let resp = draft(
        "req-1".to_string(),
        &json!({"data":"valid material text"}),
        &lookup,
        &[],
    );

    assert_eq!(resp.status, "error");
    let extra = resp.extra.expect("error extra");
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["error_kind"], "missing_person");
    assert_eq!(extra["message_key"], "skill.invest_copy.missing_person");
}

#[test]
fn bullets_non_empty_from_sample() {
    let sample =
        "本公司2024年一季度营收同比上升12%。毛利率改善。\n风险提示：海外市场波动可能影响出口业务。";
    let b = summarize_bullets(sample, 5);
    assert!(!b.is_empty());
}
