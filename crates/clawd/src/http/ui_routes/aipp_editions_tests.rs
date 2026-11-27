use super::*;

#[test]
fn editions_isolate_owners_actions_and_manual_retries_before_pagination() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("
        CREATE TABLE tasks(task_id TEXT PRIMARY KEY, principal_id TEXT, kind TEXT, channel TEXT DEFAULT 'ui',
             status TEXT DEFAULT 'succeeded', created_at TEXT DEFAULT '123', idempotency_key TEXT,
             payload_json TEXT DEFAULT '{}', result_json TEXT DEFAULT '{}', error_text TEXT);
        CREATE TABLE task_event_archive(task_id TEXT,event_json TEXT);
        CREATE TABLE task_event_stream(task_id TEXT,event_json TEXT);
        CREATE TABLE scheduled_job_runs(id INTEGER PRIMARY KEY,run_id TEXT,task_id TEXT,result_json TEXT);
        INSERT INTO tasks(task_id,principal_id,kind,payload_json) VALUES
          ('manual','u','run_skill','{\"skill_name\":\"reader\",\"args\":{\"action\":\"fetch\"}}'),
          ('timer','u','ask','{}'),('retry','u','ask','{}'),('unrelated','u','ask','{}'),
          ('foreign','v','ask','{}'),('probe','u','ask','{}');
        UPDATE tasks SET status='failed',result_json='{\"text\":\"withheld draft\"}',error_text='{\"message_key\":\"evidence_missing\"}' WHERE task_id='retry';
        INSERT INTO scheduled_job_runs VALUES(1,'timer_run','timer','{\"notification\":{\"delivery_status\":\"accepted\"}}'),
          (2,'manual_123','retry','{}');
    ").unwrap();
    for (task, skill, action) in [
        ("timer", "reader", "content.fetch"),
        ("retry", "reader", "content.fetch"),
        ("unrelated", "other", "content.fetch"),
        ("foreign", "reader", "content.fetch"),
        ("probe", "reader", "content.inspect"),
    ] {
        let event=json!({"event_kind":"tool_finished","payload":{"skill":skill,"requested_action_ref":action}}).to_string();
        for table in ["task_event_stream", "task_event_archive"] {
            db.execute(
                &format!("INSERT INTO {table} VALUES(?1,?2)"),
                rusqlite::params![task, event],
            )
            .unwrap();
        }
    }
    let actor = claw_core::types::AuthIdentity {
        role: "user".into(),
        principal_id: "u".into(),
        user_key: "test".into(),
        user_id: 1,
        chat_id: 1,
    };
    let refs = vec!["reader.fetch".into(), "content.fetch".into()];
    let query = |trigger: Option<&str>, cursor| AippMediaQuery {
        trigger: trigger.map(str::to_owned),
        cursor_sequence: cursor,
        limit: Some(1),
        ..Default::default()
    };
    let page = |filter, cursor| {
        skill_edition_page(&db, &actor, "reader", &refs, &query(filter, cursor)).unwrap()
    };
    let first = page(Some("manual"), None);
    assert_eq!(first["items"][0]["task_id"], "retry");
    assert!(first["items"][0]["text"].is_null());
    assert_eq!(first["items"][0]["error_code"], "evidence_missing");
    let last = page(Some("manual"), first["next_cursor"].as_u64());
    assert_eq!(last["items"][0]["task_id"], "manual");
    assert!(last["next_cursor"].is_null());
    let scheduled = page(Some("scheduled"), None);
    assert_eq!(scheduled["items"][0]["task_id"], "timer");
    assert_eq!(scheduled["items"][0]["delivery_status"], "accepted");
    assert!(scheduled["next_cursor"].is_null());
}

#[test]
fn edition_sources_preserve_attribution_and_exclude_unbound_or_unsafe_links() {
    let source = json!({"title":"Publisher title","link":"https://example.org/article","excerpt":"Publisher excerpt"});
    let capability = |skill| {
        json!({"capability":"content.fetch","action":"fetch","data":{"extra":{
            "execution_binding":{"skill_name":skill},"items":[source,source,{"link":"javascript:alert(1)"},{"link":"https://u:p@example.org"}]
        }}})
    };
    let refs = vec!["content.fetch".to_owned()];
    assert!(edition_sources(
        Some(&json!([capability("other")]).to_string()),
        "reader",
        &refs
    )
    .is_empty());
    let rows = edition_sources(
        Some(&json!([capability("reader")]).to_string()),
        "reader",
        &refs,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["excerpt"], "Publisher excerpt");
}
