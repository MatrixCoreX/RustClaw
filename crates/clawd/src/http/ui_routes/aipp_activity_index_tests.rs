use super::*;

const OLD_INDEXES: &str = r#"
CREATE INDEX idx_task_event_stream_skill_activity ON task_event_stream(
    json_extract(event_json, '$.payload.skill'), task_id, created_at_ms
) WHERE json_extract(event_json, '$.event_kind') = 'tool_finished';
CREATE INDEX idx_task_event_archive_skill_activity ON task_event_archive(
    json_extract(event_json, '$.payload.skill'), task_id, created_at_ms
) WHERE json_extract(event_json, '$.event_kind') = 'tool_finished';
"#;

fn indexed_db() -> rusqlite::Connection {
    let db = task_activity_db();
    db.execute_batch(OLD_INDEXES).unwrap();
    db.execute_batch(crate::task_event_transport::SKILL_EXECUTION_INDEX_SQL)
        .unwrap();
    db
}

fn legacy_match() -> String {
    let mut sql = AIPP_TASK_ACTIVITY_MATCH_SQL.to_string();
    for table in ["task_event_stream", "task_event_archive"] {
        sql = sql.replace(
            &format!("(json_extract({table}.event_json, '$.payload.skill') || '.capabilities')"),
            "(:skill_name || '.capabilities')",
        );
    }
    assert_ne!(
        sql, AIPP_TASK_ACTIVITY_MATCH_SQL,
        "baseline must use the former predicate"
    );
    sql
}

fn matching_ids(
    db: &rusqlite::Connection,
    sql: &str,
    skill: &str,
    query: &AippMediaQuery,
    communication_only: bool,
    cleared: u64,
    hidden: &[String],
) -> Vec<String> {
    let hidden = serde_json::to_string(hidden).unwrap();
    let needle = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut statement = db
        .prepare(&format!(
            "SELECT task_id FROM tasks {sql} ORDER BY tasks.rowid ASC"
        ))
        .unwrap();
    let ids = statement.query_map(rusqlite::named_params! {
        ":skill_name":skill, ":channel":query.channel.as_deref(), ":status":query.status.as_deref(),
        ":communication_only":i64::from(communication_only), ":needle":needle,
        ":cleared_through_event_ms":cleared as i64, ":hidden_json":hidden,
    }, |row| row.get(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    ids
}

fn seed_matrix(db: &rusqlite::Connection) -> Vec<String> {
    let skills = [
        Some(json!("fixture")),
        Some(json!("other")),
        Some(Value::Null),
        None,
        Some(json!(7)),
        Some(json!({"name":"fixture"})),
    ];
    let actions = [
        Some(json!("fixture.capabilities")),
        Some(json!("fixture.download")),
        Some(json!("other.capabilities")),
        None,
        Some(Value::Null),
        Some(json!("")),
        Some(json!(7)),
        Some(json!(true)),
        Some(json!({"name":"fixture.capabilities"})),
    ];
    let mut expected = Vec::new();
    let mut index = 0;
    for kind in ["tool_finished", "tool_started"] {
        for skill in &skills {
            for action in &actions {
                let task = format!("matrix-{index}");
                let channel = if index % 3 == 0 {
                    "ui"
                } else if index % 3 == 1 {
                    "wechat"
                } else {
                    "telegram"
                };
                let status = if index % 2 == 0 {
                    "succeeded"
                } else {
                    "failed"
                };
                db.execute(
                    "INSERT INTO tasks VALUES (?1,NULL,?2,?3,?4,?5,NULL,'100','101')",
                    rusqlite::params![
                        task,
                        channel,
                        status,
                        json!({"text":format!("request {task}")}).to_string(),
                        json!({"text":format!("result {task}")}).to_string()
                    ],
                )
                .unwrap();
                let mut payload = serde_json::Map::new();
                if let Some(value) = skill {
                    payload.insert("skill".into(), value.clone());
                }
                if let Some(value) = action {
                    payload.insert("requested_action_ref".into(), value.clone());
                }
                let event = json!({"event_kind":kind,"payload":payload}).to_string();
                let table = if index % 2 == 0 {
                    "task_event_stream"
                } else {
                    "task_event_archive"
                };
                db.execute(
                    &format!("INSERT INTO {table} VALUES (?1,?2,?3)"),
                    rusqlite::params![task, event, index * 10 + 1],
                )
                .unwrap();
                if index % 5 == 0 {
                    let other = if table == "task_event_stream" {
                        "task_event_archive"
                    } else {
                        "task_event_stream"
                    };
                    db.execute(
                        &format!("INSERT INTO {other} VALUES (?1,?2,?3)"),
                        rusqlite::params![task, event, index * 10 + 2],
                    )
                    .unwrap();
                }
                if kind == "tool_finished"
                    && skill.as_ref().and_then(Value::as_str) == Some("fixture")
                    && action.as_ref().and_then(Value::as_str) != Some("fixture.capabilities")
                {
                    expected.push(task);
                }
                index += 1;
            }
        }
    }
    expected
}

#[test]
fn aipp_execution_indexes_coexist_and_accelerate_membership_after_clear() {
    let db = indexed_db();
    seed_matrix(&db);
    let sql =
        format!("EXPLAIN QUERY PLAN SELECT count(*) FROM tasks {AIPP_TASK_ACTIVITY_MATCH_SQL}");
    let mut statement = db.prepare(&sql).unwrap();
    let plan = statement.query_map(rusqlite::named_params! {
        ":skill_name":"fixture", ":channel":Option::<&str>::None, ":status":Option::<&str>::None,
        ":communication_only":0, ":needle":Option::<&str>::None, ":cleared_through_event_ms":500, ":hidden_json":"[]",
    }, |row| row.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    for table in ["task_event_stream", "task_event_archive"] {
        assert!(
            plan.iter()
                .any(|line| line.contains(&format!("idx_{table}_skill_execution"))),
            "{plan:?}"
        );
        let indexes: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN (?1,?2)",
                rusqlite::params![
                    format!("idx_{table}_skill_activity"),
                    format!("idx_{table}_skill_execution")
                ],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 2);
    }
    assert!(
        !plan.iter().any(|line| line.starts_with("SCAN tasks")),
        "{plan:?}"
    );
}

#[test]
fn aipp_indexed_membership_matches_legacy_for_typed_fields_filters_and_hidden_rows() {
    let db = indexed_db();
    let expected = seed_matrix(&db);
    assert_eq!(
        matching_ids(
            &db,
            AIPP_TASK_ACTIVITY_MATCH_SQL,
            "fixture",
            &AippMediaQuery::default(),
            false,
            0,
            &[]
        ),
        expected
    );
    let legacy = legacy_match();
    let hidden = vec!["matrix-1".into(), "matrix-3".into(), "not-present".into()];
    for skill in ["fixture", "other", "7"] {
        for cleared in [0, 150, 900] {
            for communication in [false, true] {
                for query in [
                    AippMediaQuery::default(),
                    AippMediaQuery {
                        channel: Some("wechat".into()),
                        ..Default::default()
                    },
                    AippMediaQuery {
                        status: Some("failed".into()),
                        ..Default::default()
                    },
                    AippMediaQuery {
                        query: Some("result matrix-".into()),
                        ..Default::default()
                    },
                    AippMediaQuery {
                        query: Some("no match".into()),
                        ..Default::default()
                    },
                ] {
                    for excluded in [&[][..], hidden.as_slice()] {
                        assert_eq!(
                            matching_ids(
                                &db,
                                AIPP_TASK_ACTIVITY_MATCH_SQL,
                                skill,
                                &query,
                                communication,
                                cleared,
                                excluded
                            ),
                            matching_ids(
                                &db,
                                &legacy,
                                skill,
                                &query,
                                communication,
                                cleared,
                                excluded
                            )
                        );
                    }
                }
            }
        }
    }
    let all = matching_ids(
        &db,
        &legacy,
        "fixture",
        &AippMediaQuery::default(),
        false,
        0,
        &[],
    );
    let owned = owned_aipp_task_activity_ids(&db, "fixture", &all).unwrap();
    assert_eq!(
        owned,
        expected
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    );
}

#[test]
fn aipp_indexed_pages_preserve_totals_order_cursors_and_projection() {
    let db = indexed_db();
    seed_matrix(&db);
    let legacy = legacy_match();
    let hidden = vec!["matrix-1".into(), "matrix-4".into()];
    for oldest in [true, false] {
        for limit in [1, 3, 30] {
            for cleared in [0, 50] {
                let mut query = AippMediaQuery {
                    limit: Some(limit),
                    sort_order: Some(if oldest { "oldest" } else { "newest" }.into()),
                    ..Default::default()
                };
                let mut expected =
                    matching_ids(&db, &legacy, "fixture", &query, false, cleared, &hidden);
                if !oldest {
                    expected.reverse();
                }
                let mut actual = Vec::new();
                loop {
                    let page = read_aipp_task_activity_page(
                        &db, "fixture", &query, false, cleared, &hidden,
                    )
                    .unwrap();
                    assert_eq!(page["total_item_count"], expected.len());
                    let items = page["items"].as_array().unwrap();
                    assert!(items.len() <= limit);
                    for item in items {
                        let id = item["task_id"].as_str().unwrap();
                        assert_eq!(item["input_text"], format!("request {id}"));
                        assert_eq!(item["result_text"], format!("result {id}"));
                        actual.push(id.to_string());
                    }
                    assert!(
                        actual.len() <= expected.len(),
                        "pagination must not repeat rows"
                    );
                    if let Some(cursor) = page["next_cursor_sequence"].as_u64() {
                        query.cursor_sequence = Some(cursor);
                    } else {
                        break;
                    }
                }
                assert_eq!(actual, expected);
            }
        }
    }
}
