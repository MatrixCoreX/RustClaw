use rusqlite::{params, Connection};

use crate::{now_ts, AppState};

pub(crate) fn insert_audit_log(
    state: &AppState,
    user_id: Option<i64>,
    action: &str,
    detail_json: Option<&str>,
    error_text: Option<&str>,
) -> anyhow::Result<()> {
    // Phase 2.2 Stage 2: audit_logs 走独立 audit pool（独立 SQLite 文件），
    // 与任务/调度等热路径写域分开，writer 锁互不抢占。
    let db = state
        .core
        .audit_db
        .get()
        .map_err(|e| anyhow::anyhow!("audit db pool: {e}"))?;
    insert_audit_log_raw(&db, user_id, action, detail_json, error_text)
}

pub(crate) fn insert_audit_log_raw(
    db: &Connection,
    user_id: Option<i64>,
    action: &str,
    detail_json: Option<&str>,
    error_text: Option<&str>,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO audit_logs (ts, user_id, action, detail_json, error_text) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now_ts(), user_id, action, detail_json, error_text],
    )?;
    Ok(())
}
