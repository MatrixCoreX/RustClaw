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
    assert_eq!(
        resp.error_text.as_deref(),
        Some("code=missing_person field=args.person")
    );
    let extra = resp.extra.expect("error extra");
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["error_kind"], "missing_person");
    assert_eq!(extra["message_key"], "skill.invest_copy.missing_person");
}

#[test]
fn list_investors_uses_machine_header() {
    let resp = list_investors("req-list".to_string(), &[], &json!({}));

    assert_eq!(resp.status, "ok");
    assert_eq!(resp.text, "personas count=0");
    assert_eq!(resp.extra.unwrap()["action"], "list_investors");
}

#[test]
fn compliance_terms_come_from_config_metadata() {
    let cfg = InvestCopyConfig {
        compliance_sensitive_terms: vec!["稳赚".to_string(), "guaranteed return".to_string()],
    };

    let zh_match = compliance_policy_match("这个策略稳赚不赔", "", &cfg).expect("zh policy");
    assert_eq!(zh_match.term_index, 0);
    assert_eq!(zh_match.reason_code, "configured_compliance_term");

    let en_match =
        compliance_policy_match("This has a GUARANTEED RETURN.", "", &cfg).expect("en policy");
    assert_eq!(en_match.term_index, 1);
}

#[test]
fn heuristic_draft_returns_structured_rendering_contract() {
    let persona = PersonaToml {
        slug: "sample".to_string(),
        aliases: vec!["sample".to_string()],
        display_name_zh: "样例投资者".to_string(),
        display_name_en: "Sample Investor".to_string(),
        one_liner_zh: "关注事实和风险".to_string(),
        facets_zh: vec!["现金流".to_string()],
        prefer_zh: vec!["长期".to_string()],
    };
    let personas = vec![persona];
    let lookup = build_persona_lookup(&personas);

    let resp = draft(
        "req-heuristic".to_string(),
        &json!({
            "action": "draft",
            "person": "sample",
            "data": "公司2024年营收同比上升12%。毛利率改善，现金流保持稳定。",
            "use_heuristic": true
        }),
        &lookup,
        &personas,
    );

    assert_eq!(resp.status, "ok");
    assert!(resp
        .text
        .starts_with("message_key=skill.invest_copy.draft_ready"));
    let extra = resp.extra.expect("extra");
    assert_eq!(extra["message_key"], "skill.invest_copy.draft_ready");
    assert_eq!(extra["summary_mode"], "heuristic");
    assert_eq!(extra["rendering"]["requires_language_rendering"], true);
    assert!(extra["summary_bullets"]
        .as_array()
        .is_some_and(|v| !v.is_empty()));
}

#[test]
fn bullets_non_empty_from_sample() {
    let sample =
        "本公司2024年一季度营收同比上升12%。毛利率改善。\n风险提示：海外市场波动可能影响出口业务。";
    let b = summarize_bullets(sample, 5);
    assert!(!b.is_empty());
}

#[test]
fn sentence_scoring_uses_currency_markers_without_language_units() {
    assert!(score_sentence("Revenue reached CNY 120m") > score_sentence("Revenue improved"));
    assert!(score_sentence("Revenue reached ¥120m") > score_sentence("Revenue improved"));
}
