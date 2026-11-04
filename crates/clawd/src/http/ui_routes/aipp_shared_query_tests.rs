use super::*;

fn matrix_db() -> rusqlite::Connection {
    let db = task_activity_db();
    db.execute_batch(crate::task_event_transport::SKILL_EXECUTION_INDEX_SQL)
        .unwrap();
    for number in 0..43 {
        let id = format!("fixture-{:03}", number * 17 % 47);
        insert_task_activity(
            &db,
            &id,
            ["ui", "wechat", "telegram"][number % 3],
            if number % 2 == 0 {
                "failed"
            } else {
                "succeeded"
            },
            "fixture",
            if number % 5 == 0 {
                "fixture.capabilities"
            } else {
                "fixture.download"
            },
            number % 2 == 0,
        );
        db.execute(
            "UPDATE tasks SET payload_json=json_set(payload_json,'$.text',?2) WHERE task_id=?1",
            rusqlite::params![id, format!("match request {number}")],
        )
        .unwrap();
        if number % 7 == 0 {
            db.execute("UPDATE tasks SET payload_json=json_set(payload_json,'$._task_source','distributed_work') WHERE task_id=?1", [&id]).unwrap();
        } else if number % 11 == 0 {
            db.execute("UPDATE tasks SET payload_json=json_set(payload_json,'$._task_source','aipp_account_control') WHERE task_id=?1", [&id]).unwrap();
        }
        if number % 13 == 0 {
            db.execute(
                "UPDATE tasks SET idempotency_key='nni-capability:fixture' WHERE task_id=?1",
                [&id],
            )
            .unwrap();
        }
        if number % 17 == 0 {
            db.execute(
                "UPDATE tasks SET result_json='invalid result JSON' WHERE task_id=?1",
                [&id],
            )
            .unwrap();
        }
        if number % 19 == 0 {
            db.execute(
                "UPDATE tasks SET payload_json='invalid input JSON' WHERE task_id=?1",
                [&id],
            )
            .unwrap();
        }
        for table in ["task_event_stream", "task_event_archive"] {
            db.execute(
                &format!("UPDATE {table} SET created_at_ms=?2 WHERE task_id=?1"),
                rusqlite::params![id, number * 100 + 1],
            )
            .unwrap();
        }
    }
    db
}

// The deployed two-statement implementation is kept only as an independent
// test reference; production must not retain a second query path.
fn legacy_page(
    db: &rusqlite::Connection,
    scope: &str,
    cleared: u64,
    hidden: &[String],
    query: &AippMediaQuery,
) -> Value {
    let sort = aipp_media_sort_order(query).unwrap();
    let (operator, order) = if sort == "oldest" {
        (">", "ASC")
    } else {
        ("<", "DESC")
    };
    let limit = query
        .limit
        .unwrap_or(AIPP_PAGE_SIZE)
        .clamp(1, AIPP_TASK_ACTIVITY_LIMIT);
    let needle = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let hidden = serde_json::to_string(hidden).unwrap();
    let total: u64 = db.query_row(
        &format!("SELECT COUNT(*) FROM tasks {AIPP_TASK_ACTIVITY_MATCH_SQL}"),
        rusqlite::named_params! {":skill_name":"fixture", ":channel":query.channel.as_deref(),
            ":status":query.status.as_deref(), ":needle":needle, ":communication_only":i64::from(scope=="communication"),
            ":cleared_through_event_ms":cleared as i64, ":hidden_json":hidden.as_str()},
        |row| row.get(0),
    ).unwrap();
    let sql = include_str!("fixtures/aipp_activity_page_legacy.sql")
        .replace(
            "{AIPP_TASK_ACTIVITY_MATCH_SQL}",
            AIPP_TASK_ACTIVITY_MATCH_SQL,
        )
        .replace("{cursor_operator}", operator)
        .replace("{order}", order);
    let mut statement = db.prepare(&sql).unwrap();
    let mut rows = statement.query(rusqlite::named_params! {
        ":skill_name":"fixture", ":channel":query.channel.as_deref(), ":status":query.status.as_deref(),
        ":needle":needle, ":communication_only":i64::from(scope=="communication"),
        ":cleared_through_event_ms":cleared as i64, ":hidden_json":hidden.as_str(),
        ":cursor":query.cursor_sequence.or(query.before_sequence).map(|value| value as i64),
        ":limit":(limit+1) as i64,
    }).unwrap();
    let mut items = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        items.push(task_activity_item(
            row.get::<_, u64>(0).unwrap_or_default(),
            &row.get::<_, String>(1).unwrap_or_default(),
            &row.get::<_, String>(2).unwrap_or_default(),
            &row.get::<_, String>(3).unwrap_or_default(),
            &row.get::<_, String>(4).unwrap_or_default(),
            row.get::<_, Option<String>>(5).ok().flatten().as_deref(),
            row.get::<_, Option<String>>(6).ok().flatten().as_deref(),
            &row.get::<_, String>(7).unwrap_or_default(),
            &row.get::<_, String>(8).unwrap_or_default(),
            row.get::<_, u64>(9).unwrap_or_default(),
            row.get::<_, Option<String>>(10).ok().flatten().as_deref(),
        ));
    }
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next = has_more
        .then(|| items.last()?.get("sequence")?.as_u64())
        .flatten();
    let updated = items
        .iter()
        .filter_map(|item| item["event_at_ms"].as_u64())
        .max();
    json!({"schema_version":1,"page_item_count":items.len(),"total_item_count":total,
        "items":items,"sort_order":sort,"next_cursor_sequence":next,"updated_at_ms":updated})
}

#[test]
fn aipp_shared_query_preserves_legacy_projection_filters_totals_and_cursors() {
    let db = matrix_db();
    let changes = db.total_changes();
    let hidden_cases = [vec![], vec!["fixture-001".into(), "fixture-017".into()]];
    let mut cases = 0;
    for scope in ["all", "communication"] {
        for channel in [None, Some("wechat")] {
            for status in [None, Some("failed")] {
                for needle in [None, Some("match"), Some("not present")] {
                    for cleared in [0, 1500] {
                        for hidden in &hidden_cases {
                            for cursor in [None, Some(0), Some(21), Some(99999)] {
                                for limit in [1, 5] {
                                    for oldest in [false, true] {
                                        let query = AippMediaQuery {
                                            channel: channel.map(str::to_string),
                                            status: status.map(str::to_string),
                                            query: needle.map(str::to_string),
                                            cursor_sequence: cursor,
                                            limit: Some(limit),
                                            sort_order: Some(
                                                if oldest { "oldest" } else { "newest" }.into(),
                                            ),
                                            ..Default::default()
                                        };
                                        let expected =
                                            legacy_page(&db, scope, cleared, hidden, &query);
                                        let actual = read_aipp_task_activity_page(
                                            &db, "fixture", scope, cleared, hidden, &query,
                                        )
                                        .unwrap();
                                        assert_eq!(actual, expected, "case={cases}");
                                        assert!(!actual.to_string().contains("must-not-leak"));
                                        cases += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1536);
    assert_eq!(db.total_changes(), changes);
}

#[test]
fn aipp_shared_query_empty_cursor_pages_retain_total_without_phantom_items() {
    let db = matrix_db();
    for query in [
        AippMediaQuery {
            cursor_sequence: Some(0),
            ..Default::default()
        },
        AippMediaQuery {
            before_sequence: Some(0),
            ..Default::default()
        },
        AippMediaQuery {
            cursor_sequence: Some(99999),
            sort_order: Some("oldest".into()),
            ..Default::default()
        },
    ] {
        let actual = read_aipp_task_activity_page(&db, "fixture", "all", 0, &[], &query).unwrap();
        assert_eq!(actual, legacy_page(&db, "all", 0, &[], &query));
        assert!(actual["total_item_count"].as_u64().unwrap() > 0);
        assert_eq!(actual["page_item_count"], 0);
        assert_eq!(actual["items"], json!([]));
        assert!(actual["next_cursor_sequence"].is_null());
        assert!(actual["updated_at_ms"].is_null());
    }
    for limit in [0, 999] {
        let query = AippMediaQuery {
            limit: Some(limit),
            before_sequence: Some(1),
            cursor_sequence: Some(21),
            ..Default::default()
        };
        assert_eq!(
            read_aipp_task_activity_page(&db, "fixture", "all", 0, &[], &query).unwrap(),
            legacy_page(&db, "all", 0, &[], &query)
        );
    }
}

#[test]
fn aipp_shared_query_does_not_treat_nonpositive_rowids_as_empty_metadata() {
    for sequence in [0, -3] {
        let db = task_activity_db();
        insert_task_activity(
            &db,
            "fixture-row",
            "ui",
            "succeeded",
            "fixture",
            "fixture.download",
            false,
        );
        db.execute("UPDATE tasks SET rowid=?1", [sequence]).unwrap();
        let query = AippMediaQuery::default();
        let actual = read_aipp_task_activity_page(&db, "fixture", "all", 0, &[], &query).unwrap();
        assert_eq!(actual, legacy_page(&db, "all", 0, &[], &query));
        assert_eq!(actual["page_item_count"], 1);
        assert_eq!(actual["total_item_count"], 1);
        assert_eq!(actual["items"][0]["task_id"], "fixture-row");
    }
}

#[test]
fn aipp_shared_query_materializes_one_eligibility_relation_for_total_and_page() {
    let db = matrix_db();
    let mut statement = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            aipp_task_activity_page_sql(false)
        ))
        .unwrap();
    let plan = statement.query_map(rusqlite::named_params! {
        ":skill_name":"fixture", ":channel":Option::<&str>::None, ":status":Option::<&str>::None,
        ":needle":Option::<&str>::None, ":communication_only":0, ":cleared_through_event_ms":0,
        ":hidden_json":"[]", ":cursor":Option::<i64>::None, ":limit":2,
    }, |row| row.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert_eq!(
        plan.iter()
            .filter(|line| line.contains("MATERIALIZE eligible"))
            .count(),
        1,
        "{plan:?}"
    );
    assert_eq!(
        plan.iter()
            .filter(|line| line.contains("MATERIALIZE filtered"))
            .count(),
        1,
        "{plan:?}"
    );
}
