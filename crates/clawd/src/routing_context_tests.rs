use super::*;

#[test]
fn extracts_generic_anchor_from_capability_result_envelope() {
    let anchor = extract_execution_anchor(
        "ask",
        r#"{"text":"inspect the selected item"}"#,
        r#"{"text":"done","task_journal":{"trace":{"capability_results":[{"schema_version":1,"status":"ok","capability":"catalog.lookup","action":"lookup","data":{"item_id":"item-42","value":106.02},"artifacts":[{"id":"artifact-1","path":"/tmp/report.json"}],"evidence":[{"id":"ev-1","source":"catalog.lookup","locator":"catalog://item-42","metadata":{}}],"delivery":{"intent":"model_synthesis","constraints":{}}}]}}}"#,
        "1710668477",
    )
    .expect("anchor");
    assert_eq!(anchor.capability, "catalog.lookup");
    assert_eq!(anchor.action.as_deref(), Some("lookup"));
    assert_eq!(
        anchor.data.as_ref().and_then(|data| data.get("item_id")),
        Some(&serde_json::json!("item-42"))
    );
    assert_eq!(anchor.evidence_locators, vec!["catalog://item-42"]);
    assert_eq!(anchor.artifact_refs, vec!["/tmp/report.json"]);
}

#[test]
fn legacy_visible_text_does_not_create_a_semantic_anchor() {
    assert!(extract_execution_anchor(
        "ask",
        r#"{"text":"查询中芯国际今天涨跌情况"}"#,
        r#"{"text":"subtask#1 skill(stock): success [SH688981] 中芯国际 现价106.020 今开108.540 昨收108.600"}"#,
        "1710668477",
    )
    .is_none());
}

#[test]
fn extracts_run_skill_anchor_from_structured_payload() {
    let secret = "sk-test_abcdefghijklmnopqrstuvwxyz1234567890";
    let anchor = extract_execution_anchor(
        "run_skill",
        &format!(
            r#"{{"skill_name":"catalog_lookup","args":{{"action":"lookup","item_id":"item-42","api_token":"{secret}"}}}}"#
        ),
        r#"{"text":"done"}"#,
        "1710668477",
    )
    .expect("anchor");
    assert_eq!(anchor.capability, "catalog_lookup");
    assert_eq!(anchor.action.as_deref(), Some("lookup"));
    assert_eq!(
        anchor.data.as_ref().and_then(|data| data.get("item_id")),
        Some(&serde_json::json!("item-42"))
    );
    assert!(!anchor.data.as_ref().unwrap().to_string().contains(secret));

    let context = render_recent_execution_context(
        &[(
            "run_skill".to_string(),
            format!(
                r#"{{"skill_name":"catalog_lookup","args":{{"action":"lookup","item_id":"item-42","api_token":"{secret}"}}}}"#
            ),
            r#"{"text":"done"}"#.to_string(),
            "1710668477".to_string(),
        )],
        1,
    );
    assert!(!context.contains(secret));
}
