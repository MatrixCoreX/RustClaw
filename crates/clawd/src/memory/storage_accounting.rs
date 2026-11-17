use rusqlite::Connection;

/// Logical bytes owned by the runtime memory domain, not the containing SQLite file.
/// Task journals, event archives, free pages and skill-private databases have different owners.
/// Count ordinary memory tables and FTS shadow storage once, without querying virtual views.
pub(super) fn observed_memory_bytes(db: &Connection) -> anyhow::Result<u64> {
    let mut statement = db.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table'
           AND (name IN ('memories', 'long_term_memories', 'user_preferences')
                OR substr(name, 1, 7) = 'memory_')
           AND name != 'memory_storage_pressure'
           AND upper(ltrim(COALESCE(sql, ''))) NOT LIKE 'CREATE VIRTUAL TABLE%'
         ORDER BY name",
    )?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut total = 0_u64;
    for table in tables {
        let table = quote_identifier(&table);
        let mut columns = db.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = columns
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if columns.is_empty() {
            continue;
        }
        let sizes = columns
            .iter()
            .map(|column| {
                let column = quote_identifier(column);
                format!(
                    "CASE typeof({column}) WHEN 'null' THEN 0 \
                     WHEN 'integer' THEN 8 WHEN 'real' THEN 8 \
                     ELSE length(CAST({column} AS BLOB)) END"
                )
            })
            .collect::<Vec<_>>()
            .join(" + ");
        let bytes: u64 = db.query_row(
            &format!("SELECT COALESCE(SUM({sizes}), 0) FROM {table}"),
            [],
            |row| row.get(0),
        )?;
        total = total.saturating_add(bytes);
    }
    Ok(total)
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
#[path = "storage_accounting_tests.rs"]
mod tests;
