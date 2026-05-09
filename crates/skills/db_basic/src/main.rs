use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Clone)]
struct SkillError {
    kind: &'static str,
    text: String,
    extra: Option<Value>,
}

impl SkillError {
    fn new(kind: &'static str, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            extra: None,
        }
    }

    fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_kind: None,
                    platform: None,
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_kind: Some(err.kind.to_string()),
                    platform: Some(std::env::consts::OS.to_string()),
                    extra: err.extra,
                    error_text: Some(err.text),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_kind: Some("invalid_input".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
                extra: Some(json!({
                    "error_kind": "invalid_input",
                    "source": "request_json",
                })),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(args: Value) -> Result<(String, Value), SkillError> {
    let obj = args
        .as_object()
        .ok_or_else(|| SkillError::new("invalid_input", "args must be object"))?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("sqlite_query");
    let db_path = obj
        .get("db_path")
        .and_then(|v| v.as_str())
        .unwrap_or("data/rustclaw.db");
    let sql = match action {
        "schema_version" | "sqlite_schema_version" => "PRAGMA schema_version;".to_string(),
        "list_tables" | "table_names" | "sqlite_table_names" | "sqlite_tables" => {
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;".to_string()
        }
        _ => required_sql(obj)?,
    };

    let root = workspace_root();
    let db = resolve_path(&root, db_path)?;
    let conn = Connection::open(&db).map_err(|err| {
        SkillError::new("sqlite_open_failed", format!("open sqlite failed: {err}")).with_extra(
            json!({
                "error_kind": "sqlite_open_failed",
                "action": action,
                "db_path": db.display().to_string(),
            }),
        )
    })?;
    let db_path_text = db.display().to_string();

    match action {
        "schema_version" | "sqlite_schema_version" => run_query(&conn, &sql, obj).map(|payload| {
            (
                payload.to_string(),
                json!({
                    "action": "schema_version",
                    "db_path": db_path_text,
                    "sql": sql,
                    "result": payload,
                }),
            )
        }),
        "list_tables" | "table_names" | "sqlite_table_names" | "sqlite_tables" => {
            run_query(&conn, &sql, obj).map(|payload| {
                (
                    payload.to_string(),
                    json!({
                        "action": "list_tables",
                        "db_path": db_path_text,
                        "sql": sql,
                        "result": payload,
                    }),
                )
            })
        }
        "sqlite_query" => run_query(&conn, &sql, obj).map(|payload| {
            (
                payload.to_string(),
                json!({
                    "action": "sqlite_query",
                    "db_path": db_path_text,
                    "sql": sql,
                    "result": payload,
                }),
            )
        }),
        "sqlite_execute" => {
            let confirm = obj
                .get("confirm")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !confirm {
                return Err(SkillError::new(
                    "confirmation_required",
                    "sqlite_execute requires confirm=true",
                )
                .with_extra(json!({
                    "error_kind": "confirmation_required",
                    "action": "sqlite_execute",
                    "db_path": db_path_text,
                })));
            }
            let changed = conn.execute(&sql, []).map_err(|err| {
                SkillError::new("sqlite_execute_failed", format!("execute failed: {err}"))
                    .with_extra(json!({
                        "error_kind": "sqlite_execute_failed",
                        "action": "sqlite_execute",
                        "db_path": db_path_text,
                        "sql": sql,
                    }))
            })?;
            let payload = json!({"rows_affected": changed});
            Ok((
                payload.to_string(),
                json!({
                    "action": "sqlite_execute",
                    "db_path": db_path_text,
                    "sql": sql,
                    "result": payload,
                }),
            ))
        }
        _ => Err(SkillError::new(
            "unsupported_action",
            "unsupported action; use sqlite_query|sqlite_execute|schema_version|list_tables",
        )
        .with_extra(json!({
            "error_kind": "unsupported_action",
            "action": action,
            "allowed_actions": ["sqlite_query", "sqlite_execute", "schema_version", "list_tables"],
        }))),
    }
}

fn required_sql(obj: &serde_json::Map<String, Value>) -> Result<String, SkillError> {
    let sql = obj
        .get("sql")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SkillError::new("invalid_input", "sql is required"))?
        .trim()
        .to_string();
    if sql.is_empty() {
        return Err(SkillError::new("invalid_input", "sql is empty"));
    }
    Ok(sql)
}

fn run_query(
    conn: &Connection,
    sql: &str,
    obj: &serde_json::Map<String, Value>,
) -> Result<Value, SkillError> {
    let lower = sql.to_ascii_lowercase();
    if !(lower.starts_with("select") || lower.starts_with("pragma") || lower.starts_with("with")) {
        return Err(
            SkillError::new("unsafe_sql", "sqlite_query only allows SELECT/PRAGMA/WITH")
                .with_extra(json!({
                    "error_kind": "unsafe_sql",
                    "action": "sqlite_query",
                    "allowed_statement_kinds": ["SELECT", "PRAGMA", "WITH"],
                })),
        );
    }

    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(500) as usize;

    let mut stmt = conn.prepare(sql).map_err(|err| {
        SkillError::new(
            "sqlite_query_failed",
            format!("prepare query failed: {err}"),
        )
    })?;
    let col_count = stmt.column_count();
    let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query([]).map_err(|err| {
        SkillError::new("sqlite_query_failed", format!("run query failed: {err}"))
    })?;
    let mut out_rows = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| SkillError::new("sqlite_query_failed", format!("next row failed: {err}")))?
    {
        let mut item = serde_json::Map::new();
        for i in 0..col_count {
            let key = columns
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("col_{i}"));
            let value = row.get_ref(i).map_err(|err| {
                SkillError::new("sqlite_query_failed", format!("read col failed: {err}"))
            })?;
            item.insert(key, sql_value_to_json(value));
        }
        out_rows.push(Value::Object(item));
        if out_rows.len() >= limit {
            break;
        }
    }
    Ok(json!({"columns": columns, "rows": out_rows}))
}

fn sql_value_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(s) => json!(String::from_utf8_lossy(s).to_string()),
        ValueRef::Blob(b) => json!(format!("BLOB({} bytes)", b.len())),
    }
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, SkillError> {
    let raw = Path::new(input);
    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => {
                return Err(SkillError::new(
                    "path_outside_workspace",
                    "path with '..' is not allowed",
                )
                .with_extra(json!({
                    "error_kind": "path_outside_workspace",
                    "db_path": input,
                })))
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if raw.is_absolute() {
        return Ok(normalized);
    }
    Ok(workspace_root.join(normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
