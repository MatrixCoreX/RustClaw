use super::*;

#[test]
fn reconciliation_never_takes_latest_chat_rows_from_a_different_turn() {
    let db = super::super::tests::setup_db();
    db.execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at)
        VALUES ('verified-source',1,2,'fixture-key','principal-fixture','ui','ask','{\"text\":\"authentic user input\"}','succeeded','100','100')", []).unwrap();
    db.execute(
        "UPDATE tasks SET result_json = ?1 WHERE task_id = 'verified-source'",
        [json!({"text":"authentic answer"}).to_string()],
    )
    .unwrap();
    for (id, role, text, time) in [
        (1, "user", "authentic user input", 100),
        (2, "assistant", "authentic answer", 101),
        (3, "user", "unrelated newer input", 200),
        (4, "assistant", "unrelated answer", 201),
    ] {
        db.execute("INSERT INTO memories(id,user_id,chat_id,user_key,principal_id,channel,role,content,created_at,created_at_ts)
            VALUES (?1,1,2,'fixture-key','principal-fixture','ui',?2,?3,'100',?4)", params![id, role, text, time]).unwrap();
    }
    let task = super::super::tests::fixture_task("verified-source");
    assert_eq!(
        verified_ids(&db, &task, "principal-fixture").unwrap(),
        Some((1, 2))
    );
    assert_eq!(verified_ids(&db, &task, "another-principal").unwrap(), None);
    db.execute(
        "UPDATE memories SET content='wrong nearby answer' WHERE id=2",
        [],
    )
    .unwrap();
    assert_eq!(verified_ids(&db, &task, "principal-fixture").unwrap(), None);
    db.execute(
        "UPDATE memories SET content='authentic answer' WHERE id=2",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE memories SET content='different input' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(verified_ids(&db, &task, "principal-fixture").unwrap(), None);
}

#[test]
fn ambiguous_nearby_answers_are_not_guessed() {
    let db = super::super::tests::setup_db();
    db.execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at)
        VALUES ('ambiguous-source',1,2,'fixture-key','principal-fixture','ui','ask','{\"text\":\"input\"}','succeeded','100','100')", []).unwrap();
    db.execute(
        "UPDATE tasks SET result_json = ?1 WHERE task_id = 'ambiguous-source'",
        [json!({"text":"answer"}).to_string()],
    )
    .unwrap();
    for (id, role, text) in [
        (1, "user", "input"),
        (2, "assistant", "answer"),
        (3, "assistant", "answer"),
    ] {
        db.execute("INSERT INTO memories(id,user_id,chat_id,user_key,principal_id,channel,role,content,created_at,created_at_ts)
            VALUES (?1,1,2,'fixture-key','principal-fixture','ui',?2,?3,'100',100)", params![id,role,text]).unwrap();
    }
    let task = super::super::tests::fixture_task("ambiguous-source");
    assert_eq!(verified_ids(&db, &task, "principal-fixture").unwrap(), None);
}
