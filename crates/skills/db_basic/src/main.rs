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
    error_text: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok(text) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("sqlite_query");
    let db_path = obj
        .get("db_path")
        .and_then(|v| v.as_str())
        .unwrap_or("data/rustclaw.db");
    let sql = obj
        .get("sql")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "sql is required".to_string())?
        .trim()
        .to_string();
    if sql.is_empty() {
        return Err("sql is empty".to_string());
    }

    let root = workspace_root();
    let db = resolve_path(&root, db_path)?;
    let conn = Connection::open(db).map_err(|err| format!("open sqlite failed: {err}"))?;

    match action {
        "sqlite_query" => run_query(&conn, &sql, obj),
        "sqlite_execute" => {
            let confirm = obj.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);
            if !confirm {
                return Err("sqlite_execute requires confirm=true".to_string());
            }
            let changed = conn
                .execute(&sql, [])
                .map_err(|err| format!("execute failed: {err}"))?;
            Ok(json!({"rows_affected": changed}).to_string())
        }
        _ => Err("unsupported action; use sqlite_query|sqlite_execute".to_string()),
    }
}

fn run_query(
    conn: &Connection,
    sql: &str,
    obj: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let lower = sql.to_ascii_lowercase();
    if !(lower.starts_with("select") || lower.starts_with("pragma") || lower.starts_with("with")) {
        return Err("sqlite_query only allows SELECT/PRAGMA/WITH".to_string());
    }

    let limit = obj
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(500) as usize;

    let mut stmt = conn
        .prepare(sql)
        .map_err(|err| format!("prepare query failed: {err}"))?;
    let col_count = stmt.column_count();
    let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query([]).map_err(|err| format!("run query failed: {err}"))?;
    let mut out_rows = Vec::new();
    while let Some(row) = rows.next().map_err(|err| format!("next row failed: {err}"))? {
        let mut item = serde_json::Map::new();
        for i in 0..col_count {
            let key = columns
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("col_{i}"));
            let value = row.get_ref(i).map_err(|err| format!("read col failed: {err}"))?;
            item.insert(key, sql_value_to_json(value));
        }
        out_rows.push(Value::Object(item));
        if out_rows.len() >= limit {
            break;
        }
    }
    Ok(json!({"columns": columns, "rows": out_rows}).to_string())
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

fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let base = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        workspace_root.join(input)
    };
    if base.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path with '..' is not allowed".to_string());
    }
    if !base.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }
    Ok(base)
}
