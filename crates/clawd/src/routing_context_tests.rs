use super::*;
use rusqlite::params;

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

#[test]
fn newer_non_execution_turn_invalidates_older_execution_anchor() {
    let rows = vec![
        (
            "ask".to_string(),
            r#"{"text":"target-beta"}"#.to_string(),
            r#"{"text":"which operation"}"#.to_string(),
            "200".to_string(),
        ),
        (
            "ask".to_string(),
            r#"{"text":"target-alpha"}"#.to_string(),
            r#"{"text":"done","task_journal":{"trace":{"capability_results":[{"schema_version":1,"status":"ok","capability":"catalog.lookup","action":"lookup","data":{"item_id":"target-alpha"}}]}}}"#.to_string(),
            "100".to_string(),
        ),
    ];

    assert_eq!(render_recent_execution_anchor_context(&rows), "<none>");
    let context = render_recent_execution_context(&rows, 8);
    assert!(context.contains("target-beta"));
    assert!(context.contains("target-alpha"));
    assert!(!context.contains("latest_capability="));
}

#[test]
fn recent_execution_rows_are_scoped_to_current_conversation() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            result_json TEXT NOT NULL,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("create tasks");
    for (task_id, conversation_id, target, updated_at) in [
        (
            "task-selected",
            "conversation-selected",
            "target-beta",
            "200",
        ),
        ("task-other", "conversation-other", "target-gamma", "300"),
    ] {
        db.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, user_key, kind, payload_json, result_json, status, updated_at)
             VALUES (?1, 1, 2, 'test-user', 'ask', ?2, ?3, 'succeeded', ?4)",
            params![
                task_id,
                serde_json::json!({
                    "conversation_id": conversation_id,
                    "text": target,
                })
                .to_string(),
                serde_json::json!({ "text": format!("completed-{target}") }).to_string(),
                updated_at,
            ],
        )
        .expect("insert task");
    }

    let rows = query_recent_execution_rows(
        &state,
        &db,
        1,
        2,
        Some("test-user"),
        Some("conversation-selected"),
        8,
    )
    .expect("query recent rows");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].1.contains("target-beta"));
    assert!(!rows[0].1.contains("target-gamma"));
}
