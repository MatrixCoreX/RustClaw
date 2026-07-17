use rusqlite::{params, OptionalExtension};

use crate::db_init::DbPool;
use crate::providers::LlmCallCostRecord;
use crate::ClaimedTask;

const INIT_LLM_COST_LEDGER_SQL: &str = "
CREATE TABLE IF NOT EXISTS llm_cost_ledger (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id                  TEXT NOT NULL,
    user_id                  INTEGER NOT NULL,
    provider                 TEXT NOT NULL,
    model                    TEXT NOT NULL,
    logical_call_index       INTEGER NOT NULL,
    prompt_label             TEXT NOT NULL,
    provider_status          TEXT NOT NULL,
    cost_status              TEXT NOT NULL,
    estimated_cost_usd_nanos INTEGER,
    record_json              TEXT NOT NULL,
    created_at_ts            INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_cost_ledger_task_id
    ON llm_cost_ledger(task_id, id);
CREATE INDEX IF NOT EXISTS idx_llm_cost_ledger_user_created
    ON llm_cost_ledger(user_id, created_at_ts);
CREATE INDEX IF NOT EXISTS idx_llm_cost_ledger_provider_created
    ON llm_cost_ledger(provider, created_at_ts);
";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct CostLedgerSpend {
    pub(super) known_cost_usd_nanos: u64,
    pub(super) unknown_record_count: u64,
}

pub(super) fn append_record(
    pool: &DbPool,
    task: &ClaimedTask,
    record: &LlmCallCostRecord,
) -> Result<(), String> {
    let conn = connection(pool)?;
    ensure_schema(&conn)?;
    let record_json =
        serde_json::to_string(record).map_err(|err| format!("serialize_llm_cost_record:{err}"))?;
    conn.execute(
        "INSERT INTO llm_cost_ledger (
            task_id, user_id, provider, model, logical_call_index, prompt_label,
            provider_status, cost_status, estimated_cost_usd_nanos, record_json, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            task.task_id,
            task.user_id,
            record.provider,
            record.model,
            record.logical_call_index,
            record.prompt_label,
            record.provider_status,
            record.cost_status,
            record.estimated_cost_usd_nanos,
            record_json,
            crate::now_ts_u64().min(i64::MAX as u64) as i64,
        ],
    )
    .map_err(|err| format!("insert_llm_cost_record:{err}"))?;
    Ok(())
}

pub(super) fn task_records(pool: &DbPool, task_id: &str) -> Result<Vec<LlmCallCostRecord>, String> {
    let conn = connection(pool)?;
    ensure_schema(&conn)?;
    let mut stmt = conn
        .prepare("SELECT record_json FROM llm_cost_ledger WHERE task_id=?1 ORDER BY id")
        .map_err(|err| format!("prepare_llm_cost_records:{err}"))?;
    let rows = stmt
        .query_map([task_id], |row| row.get::<_, String>(0))
        .map_err(|err| format!("query_llm_cost_records:{err}"))?;
    let mut records = Vec::new();
    for row in rows {
        let raw = row.map_err(|err| format!("read_llm_cost_record:{err}"))?;
        records.push(
            serde_json::from_str(&raw).map_err(|err| format!("parse_llm_cost_record:{err}"))?,
        );
    }
    Ok(records)
}

pub(super) fn max_logical_call_index(pool: &DbPool, task_id: &str) -> Result<u64, String> {
    let conn = connection(pool)?;
    ensure_schema(&conn)?;
    conn.query_row(
        "SELECT MAX(logical_call_index) FROM llm_cost_ledger WHERE task_id=?1",
        [task_id],
        |row| row.get::<_, Option<u64>>(0),
    )
    .optional()
    .map_err(|err| format!("query_llm_cost_call_index:{err}"))
    .map(|value| value.flatten().unwrap_or(0))
}

pub(super) fn task_spend(pool: &DbPool, task_id: &str) -> Result<CostLedgerSpend, String> {
    spend_query(
        pool,
        "SELECT COALESCE(SUM(estimated_cost_usd_nanos), 0),
                SUM(CASE WHEN estimated_cost_usd_nanos IS NULL THEN 1 ELSE 0 END)
         FROM llm_cost_ledger WHERE task_id=?1",
        rusqlite::params![task_id],
    )
}

pub(super) fn user_spend_since(
    pool: &DbPool,
    user_id: i64,
    since: i64,
) -> Result<CostLedgerSpend, String> {
    spend_query(
        pool,
        "SELECT COALESCE(SUM(estimated_cost_usd_nanos), 0),
                SUM(CASE WHEN estimated_cost_usd_nanos IS NULL THEN 1 ELSE 0 END)
         FROM llm_cost_ledger WHERE user_id=?1 AND created_at_ts>=?2",
        rusqlite::params![user_id, since],
    )
}

pub(super) fn provider_spend_since(
    pool: &DbPool,
    provider: &str,
    since: i64,
) -> Result<CostLedgerSpend, String> {
    spend_query(
        pool,
        "SELECT COALESCE(SUM(estimated_cost_usd_nanos), 0),
                SUM(CASE WHEN estimated_cost_usd_nanos IS NULL THEN 1 ELSE 0 END)
         FROM llm_cost_ledger WHERE provider=?1 AND created_at_ts>=?2",
        rusqlite::params![provider, since],
    )
}

fn spend_query(
    pool: &DbPool,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<CostLedgerSpend, String> {
    let conn = connection(pool)?;
    ensure_schema(&conn)?;
    conn.query_row(sql, params, |row| {
        Ok(CostLedgerSpend {
            known_cost_usd_nanos: row.get(0)?,
            unknown_record_count: row.get::<_, Option<u64>>(1)?.unwrap_or(0),
        })
    })
    .map_err(|err| format!("query_llm_cost_spend:{err}"))
}

fn connection(
    pool: &DbPool,
) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>, String> {
    pool.get()
        .map_err(|err| format!("acquire_llm_cost_ledger_connection:{err}"))
}

fn ensure_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(INIT_LLM_COST_LEDGER_SQL)
        .map_err(|err| format!("ensure_llm_cost_ledger_schema:{err}"))
}
