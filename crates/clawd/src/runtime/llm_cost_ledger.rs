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

/// Fill only recorded usage for which an operator catalog covers the date of the
/// call. Retain the original ledger projection alongside every recalculation.
pub(crate) fn reconcile_catalog(
    db: &rusqlite::Connection,
    catalog: &[claw_core::config::LlmModelPricingConfig],
) -> anyhow::Result<(usize, usize)> {
    db.execute_batch(INIT_LLM_COST_LEDGER_SQL)?;
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS llm_cost_pricing_revisions (
        ledger_id INTEGER PRIMARY KEY, original_record_json TEXT NOT NULL, repriced_at_ts INTEGER NOT NULL
    )")?;
    let mut changed = 0;
    let mut unavailable = 0;
    {
        let mut stmt = tx.prepare("SELECT id,record_json,date(created_at_ts,'unixepoch') FROM llm_cost_ledger
            WHERE cost_status='unknown' AND json_valid(record_json) AND json_extract(record_json,'$.unknown_reason')='pricing_not_configured' ORDER BY id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let original: String = row.get(1)?;
            let date: Option<String> = row.get(2)?;
            let record: LlmCallCostRecord = match serde_json::from_str(&original) {
                Ok(record) => record,
                Err(_) => {
                    unavailable += 1;
                    continue;
                }
            };
            let eligible = date.as_deref().and_then(|date| {
                crate::providers::model_pricing_on(
                    catalog,
                    &record.provider,
                    "",
                    &record.model,
                    date,
                )
            });
            let Some(price) = eligible else {
                unavailable += 1;
                continue;
            };
            let revised = crate::providers::build_cost_record(
                record.logical_call_index,
                &record.prompt_label,
                &record.provider,
                &record.model,
                &record.provider_status,
                record.provider_attempts as usize,
                record.usage.as_ref(),
                Some(price),
            );
            let Some(nanos) = revised.estimated_cost_usd_nanos else {
                unavailable += 1;
                continue;
            };
            tx.execute(
                "INSERT INTO llm_cost_pricing_revisions VALUES (?1,?2,?3)",
                params![id, original, crate::now_ts_u64() as i64],
            )?;
            tx.execute("UPDATE llm_cost_ledger SET record_json=?2,cost_status='known',estimated_cost_usd_nanos=?3 WHERE id=?1",
                params![id,serde_json::to_string(&revised)?,i64::try_from(nanos)?])?;
            changed += 1;
        }
    }
    tx.commit()?;
    Ok((changed, unavailable))
}

#[cfg(test)]
mod pricing_reconciliation_tests {
    use super::*;

    #[test]
    fn historical_pricing_requires_both_usage_and_a_catalog_effective_on_the_call_date() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(INIT_LLM_COST_LEDGER_SQL).unwrap();
        let cfg: claw_core::config::LlmModelPricingConfig = toml::from_str(
            r#"
            provider="vendor"
            model="example"
            currency="USD"
            effective_from="2024-01-01"
            input_usd_per_million=0.6
            output_usd_per_million=2.4
        "#,
        )
        .unwrap();
        let record = serde_json::json!({"provider":"vendor","model":"example","prompt_label":"fixture","logical_call_index":1,
            "provider_attempts":1,"provider_status":"ok","usage":{"input_tokens":100,"output_tokens":10},
            "cost_status":"unknown","unknown_reason":"pricing_not_configured"});
        for (id, ts) in [(1, 1), (2, 1_740_000_000)] {
            db.execute("INSERT INTO llm_cost_ledger(id,task_id,user_id,provider,model,logical_call_index,prompt_label,provider_status,cost_status,record_json,created_at_ts)
                VALUES(?1,'task',1,'vendor','example',1,'fixture','ok','unknown',?2,?3)", params![id,record.to_string(),ts]).unwrap();
        }
        assert_eq!(
            reconcile_catalog(&db, std::slice::from_ref(&cfg)).unwrap(),
            (1, 1)
        );
        assert_eq!(reconcile_catalog(&db, &[cfg]).unwrap(), (0, 1));
        let row: (i64, i64) = db
            .query_row(
                "SELECT estimated_cost_usd_nanos,(SELECT count(*) FROM llm_cost_pricing_revisions)
            FROM llm_cost_ledger WHERE id=2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, (84_000, 1));
    }
}
