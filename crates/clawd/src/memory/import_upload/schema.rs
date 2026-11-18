use super::*;

const ID: &str = "032_memory_import_uploads_v1";
const SQL: &str = include_str!("../../../../../migrations/032_memory_import_uploads.sql");

pub(crate) fn initialize_schema(db: &Connection) -> anyhow::Result<()> {
    super::super::ux::atomic_mutation(db, |db| {
        let applied = applied(db)?;
        if let Some(applied) = applied {
            anyhow::ensure!(
                applied == digest(SQL.as_bytes()),
                UploadError::SchemaUnavailable
            );
            db.prepare("SELECT upload_id,principal_id,file_digest,total_bytes,received_bytes,chunk_bytes,state,
                revision,parser_token,parser_lease_until_ts,preview_import_id,created_at_ts,updated_at_ts
                FROM memory_import_uploads LIMIT 0")?;
            db.prepare("SELECT upload_id,byte_offset,byte_count,content_digest,content FROM memory_import_upload_chunks LIMIT 0")?;
            db.prepare(
                "SELECT upload_id,item_index,item_json FROM memory_import_upload_items LIMIT 0",
            )?;
            return Ok(());
        }
        let existing: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name IN
             ('memory_import_uploads','memory_import_upload_chunks','memory_import_upload_items'))",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(!existing, UploadError::SchemaUnavailable);
        db.execute_batch(SQL)?;
        db.execute("INSERT INTO runtime_schema_migrations(migration_id,schema_digest,applied_at) VALUES(?1,?2,?3)",
            params![ID,digest(SQL.as_bytes()),crate::now_ts()])?;
        Ok(())
    })
}

fn applied(db: &Connection) -> anyhow::Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id=?1",
            [ID],
            |row| row.get(0),
        )
        .optional()?)
}

pub(super) fn require_schema(db: &Connection) -> anyhow::Result<()> {
    anyhow::ensure!(
        applied(db).ok().flatten().as_deref() == Some(digest(SQL.as_bytes()).as_str()),
        UploadError::SchemaUnavailable
    );
    Ok(())
}
