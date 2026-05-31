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
    let sql = normalize_readonly_sql(&sql);

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

fn normalize_readonly_sql(sql: &str) -> String {
    rewrite_table_valued_pragma_table_info(sql)
}

fn rewrite_table_valued_pragma_table_info(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut cursor = 0;
    let mut rewritten = String::new();
    let mut changed = false;

    while let Some(rel) = lower[cursor..].find("pragma") {
        let start = cursor + rel;
        let after_pragma = start + "pragma".len();
        if start > 0 && is_sql_ident_byte(bytes[start - 1]) {
            cursor = after_pragma;
            continue;
        }
        if after_pragma >= bytes.len() || !bytes[after_pragma].is_ascii_whitespace() {
            cursor = after_pragma;
            continue;
        }

        let mut idx = skip_ascii_ws(bytes, after_pragma);
        if !lower[idx..].starts_with("table_info") {
            cursor = after_pragma;
            continue;
        }
        let after_table_info = idx + "table_info".len();
        if after_table_info < bytes.len() && is_sql_ident_byte(bytes[after_table_info]) {
            cursor = after_pragma;
            continue;
        }
        idx = skip_ascii_ws(bytes, after_table_info);
        if idx >= bytes.len() || bytes[idx] != b'(' {
            cursor = after_pragma;
            continue;
        }
        if !previous_sql_context_allows_table_valued_pragma(sql, start) {
            cursor = after_pragma;
            continue;
        }

        rewritten.push_str(&sql[cursor..start]);
        rewritten.push_str("pragma_table_info");
        cursor = idx;
        changed = true;
    }

    if changed {
        rewritten.push_str(&sql[cursor..]);
        rewritten
    } else {
        sql.to_string()
    }
}

fn skip_ascii_ws(bytes: &[u8], mut idx: usize) -> usize {
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx += 1;
    }
    idx
}

fn is_sql_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn previous_sql_context_allows_table_valued_pragma(sql: &str, start: usize) -> bool {
    let prefix = sql[..start].trim_end();
    if prefix.ends_with(',') {
        return true;
    }
    let token = prefix
        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(token.as_str(), "from" | "join")
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
#[path = "main_tests.rs"]
mod tests;
