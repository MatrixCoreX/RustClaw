use super::*;

#[test]
fn task_activity_pages_do_not_evaluate_off_page_result_columns() {
    for oldest in [false, true] {
        for limit in [1_usize, 5] {
            for cursor in [None, Some(21_u64)] {
                let db = task_activity_db();
                for index in 1..=41 {
                    // Make primary-key traversal visit off-page rows before
                    // the requested rowid order, in both sorting directions.
                    let suffix = if oldest { 42 - index } else { index };
                    insert_task_activity(
                        &db,
                        &format!("fixture-{suffix:03}"),
                        "ui",
                        "succeeded",
                        "fixture",
                        "fixture.download",
                        index % 2 == 0,
                    );
                }
                db.execute(
                    "UPDATE tasks SET result_json = json_set(result_json, '$.padding', ?1)",
                    ["x".repeat(16 * 1024)],
                )
                .unwrap();
                let query = AippMediaQuery {
                    limit: Some(limit),
                    sort_order: Some(if oldest { "oldest" } else { "newest" }.into()),
                    cursor_sequence: cursor,
                    ..Default::default()
                };
                let expected =
                    read_aipp_task_activity_page(&db, "fixture", "all", 0, &[], &query).unwrap();
                let (operator, order) = if oldest { (">", "ASC") } else { ("<", "DESC") };
                let allowed = {
                    let mut statement = db
                        .prepare(&format!(
                        "SELECT rowid FROM tasks WHERE (:cursor IS NULL OR rowid {operator} :cursor)
                         ORDER BY rowid {order} LIMIT :limit"
                    ))
                        .unwrap();
                    let identities = statement.query_map(rusqlite::named_params! {
                        ":cursor":cursor.map(|value| value as i64), ":limit":(limit + 1) as i64,
                    }, |row| row.get::<_, u64>(0)).unwrap()
                        .collect::<Result<Vec<_>, _>>().unwrap();
                    identities
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                };
                db.execute_batch(&format!(
                    "ALTER TABLE tasks RENAME TO stored_tasks;
                     CREATE VIEW tasks AS
                     SELECT rowid AS rowid, task_id, idempotency_key, channel, status, payload_json,
                            CASE WHEN rowid IN ({allowed}) THEN result_json
                                 ELSE json_extract('deliberately invalid JSON', '$') END AS result_json,
                            error_text, created_at, updated_at
                       FROM stored_tasks;"
                )).unwrap();
                let changes = db.total_changes();
                let actual =
                    read_aipp_task_activity_page(&db, "fixture", "all", 0, &[], &query).unwrap();
                assert_eq!(
                    actual, expected,
                    "oldest={oldest}, limit={limit}, cursor={cursor:?}"
                );
                assert_eq!(db.total_changes(), changes);

                // Prove that the fixture really rejects an off-page read;
                // success above cannot come from an inert poison expression.
                let off_page = if oldest { 41 } else { 1 };
                assert!(db
                    .query_row(
                        "SELECT result_json FROM tasks WHERE rowid = ?1",
                        [off_page],
                        |row| row.get::<_, String>(0),
                    )
                    .is_err());
            }
        }
    }
}
