use super::*;

#[test]
fn error_extra_merges_machine_contract_and_details() {
    let extra = error_extra_with_details(
        "sqlite_open_failed",
        Some(json!({
            "path": "/tmp/missing.sqlite",
            "source": "sqlite"
        })),
    );

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "sqlite_open_failed");
    assert_eq!(extra["message_key"], "skill.db_basic.sqlite_open_failed");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["path"], "/tmp/missing.sqlite");
    assert_eq!(extra["source"], "sqlite");
}

fn temp_db_path(name: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "rustclaw-db-basic-{name}-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path.display().to_string()
}

#[test]
fn sqlite_query_rejects_mutating_sql_with_structured_kind() {
    let db_path = temp_db_path("unsafe-sql");
    let err = execute(json!({
        "action": "sqlite_query",
        "db_path": db_path,
        "sql": "DELETE FROM demo"
    }))
    .expect_err("mutating sqlite_query should fail");

    assert_eq!(err.kind, "unsafe_sql");
    assert_eq!(
        err.extra
            .as_ref()
            .and_then(|v| v.get("error_kind"))
            .and_then(Value::as_str),
        Some("unsafe_sql")
    );
}

#[test]
fn sqlite_execute_without_confirm_reports_confirmation_required() {
    let db_path = temp_db_path("confirm");
    let err = execute(json!({
        "action": "sqlite_execute",
        "db_path": db_path,
        "sql": "CREATE TABLE demo(id INTEGER)"
    }))
    .expect_err("sqlite_execute without confirm should fail");

    assert_eq!(err.kind, "confirmation_required");
    assert_eq!(
        err.extra
            .as_ref()
            .and_then(|v| v.get("error_kind"))
            .and_then(Value::as_str),
        Some("confirmation_required")
    );
}

#[test]
fn missing_sql_reports_invalid_input() {
    let err = execute(json!({"action": "sqlite_query"}))
        .expect_err("missing sql should fail before touching sqlite");

    assert_eq!(err.kind, "invalid_input");
}

#[test]
fn schema_version_action_runs_pragma_without_sql_arg() {
    let db_path = temp_db_path("schema-version");
    let (text, extra) = execute(json!({
        "action": "schema_version",
        "db_path": db_path,
    }))
    .expect("schema_version should run PRAGMA schema_version without a sql arg");

    let value: Value = serde_json::from_str(&text).expect("text should be result json");
    assert_eq!(
        extra.get("action").and_then(Value::as_str),
        Some("schema_version")
    );
    assert_eq!(
        value
            .get("columns")
            .and_then(Value::as_array)
            .and_then(|columns| columns.first())
            .and_then(Value::as_str),
        Some("schema_version")
    );
    assert_eq!(extra.get("schema_version").and_then(Value::as_i64), Some(0));
    assert_eq!(
        extra
            .pointer("/field_value/schema_version")
            .and_then(Value::as_i64),
        Some(0)
    );
}

#[test]
fn list_tables_action_runs_internal_catalog_query_without_sql_arg() {
    let db_path = temp_db_path("list-tables");
    execute(json!({
        "action": "sqlite_execute",
        "db_path": db_path,
        "sql": "CREATE TABLE demo(id INTEGER)",
        "confirm": true,
    }))
    .expect("setup table");

    let (text, extra) = execute(json!({
        "action": "list_tables",
        "db_path": db_path,
    }))
    .expect("list_tables should not require caller-provided SQL");

    let value: Value = serde_json::from_str(&text).expect("text should be result json");
    assert_eq!(
        extra.get("action").and_then(Value::as_str),
        Some("list_tables")
    );
    assert_eq!(
        value
            .get("rows")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(|row| row.get("name"))
            .and_then(Value::as_str),
        Some("demo")
    );
    assert_eq!(extra.get("table_count").and_then(Value::as_u64), Some(1));
    assert_eq!(
        extra
            .get("tables")
            .and_then(Value::as_array)
            .and_then(|tables| tables.first())
            .and_then(Value::as_str),
        Some("demo")
    );
    assert_eq!(
        extra
            .pointer("/field_value/table_count")
            .and_then(Value::as_u64),
        Some(1)
    );
}

#[test]
fn normalizes_table_valued_pragma_table_info_in_select_from_context() {
    let sql = "SELECT name FROM PRAGMA table_info('users')";
    assert_eq!(
        normalize_readonly_sql(sql),
        "SELECT name FROM pragma_table_info('users')"
    );
    let standalone = "PRAGMA table_info('users')";
    assert_eq!(normalize_readonly_sql(standalone), standalone);
}

#[test]
fn sqlite_query_accepts_union_over_table_valued_pragma_alias() {
    let db_path = temp_db_path("pragma-table-info-union");
    execute(json!({
        "action": "sqlite_execute",
        "db_path": db_path,
        "sql": "CREATE TABLE users(id INTEGER, name TEXT, email TEXT)",
        "confirm": true,
    }))
    .expect("setup users");
    execute(json!({
        "action": "sqlite_execute",
        "db_path": db_path,
        "sql": "CREATE TABLE orders(id INTEGER, amount REAL, status TEXT)",
        "confirm": true,
    }))
    .expect("setup orders");

    let (text, extra) = execute(json!({
        "action": "sqlite_query",
        "db_path": db_path,
        "sql": "SELECT tbl, name, type FROM (
            SELECT 'users' AS tbl, name, type FROM PRAGMA table_info('users')
            UNION ALL
            SELECT 'orders' AS tbl, name, type FROM PRAGMA table_info('orders')
        ) WHERE type LIKE '%TEXT%' ORDER BY tbl, name",
    }))
    .expect("sqlite_query should normalize table-valued PRAGMA syntax");

    let value: Value = serde_json::from_str(&text).expect("text should be result json");
    let rows = value.get("rows").and_then(Value::as_array).expect("rows");
    let names = rows
        .iter()
        .filter_map(|row| {
            Some(format!(
                "{}.{}",
                row.get("tbl")?.as_str()?,
                row.get("name")?.as_str()?
            ))
        })
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["orders.status", "users.email", "users.name"]);
    assert!(extra
        .get("sql")
        .and_then(Value::as_str)
        .is_some_and(|sql| sql.contains("pragma_table_info")));
}
