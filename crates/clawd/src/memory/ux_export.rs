use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::{
    memory_list_item, memory_union_sql, require_memory_ux_schema, MemoryExport,
    MemoryMarkdownExport,
};

pub(crate) fn export_memory(
    db: &Connection,
    principal_id: &str,
    now_ts: i64,
) -> anyhow::Result<MemoryExport> {
    require_memory_ux_schema(db)?;
    // One statement gives all kinds the same SQLite read snapshot. List-page
    // limits and inter-page concurrent writes cannot silently truncate an export.
    let mut statement = db.prepare(&format!(
        "SELECT * FROM ({}) item WHERE principal_id = ?1 ORDER BY updated_at_ts DESC, id DESC",
        memory_union_sql()
    ))?;
    let rows = statement.query_map([principal_id], |row| {
        memory_list_item(row, now_ts.saturating_sub(30 * 86_400))
    })?;
    let items = rows.collect::<Result<Vec<_>, _>>()?;
    let digest_input = serde_json::to_vec(&items)?;
    Ok(MemoryExport {
        schema_version: 1,
        exported_at_ts: now_ts,
        scope_kind: "principal".to_string(),
        items,
        checksum: format!("sha256:{:x}", Sha256::digest(digest_input)),
    })
}

pub(crate) fn export_memory_markdown(
    db: &Connection,
    principal_id: &str,
    now_ts: i64,
) -> anyhow::Result<MemoryMarkdownExport> {
    let export = export_memory(db, principal_id, now_ts)?;
    let mut content = String::from("# memory_export_v1\n\n");
    content.push_str(&format!("schema_version={}  \n", export.schema_version));
    content.push_str(&format!("exported_at_unix={}\n\n", export.exported_at_ts));
    for item in &export.items {
        content.push_str(&format!(
            "## {} · {}\n\n- scope_kind={}\n- origin={}\n- status={}\n- updated_at_unix={}\n- evidence_available={}\n\n{}\n\n",
            item.kind,
            item.id,
            item.scope_kind,
            item.origin,
            item.status,
            item.updated_at_ts,
            item.evidence_available,
            item.content.replace('\n', "  \n")
        ));
    }
    let checksum = format!("sha256:{:x}", Sha256::digest(content.as_bytes()));
    Ok(MemoryMarkdownExport {
        schema_version: 1,
        exported_at_ts: now_ts,
        content_type: "text/markdown; charset=utf-8".to_string(),
        content,
        checksum,
    })
}
