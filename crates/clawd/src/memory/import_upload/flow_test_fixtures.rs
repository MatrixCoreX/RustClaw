use super::*;
use serde_json::json;

pub(super) fn item(content: &str, kind: &str) -> MemoryListItem {
    serde_json::from_value(json!({"id":"source_record","revision":1,"kind":kind,"scope_kind":"principal",
        "origin":"explicit","status":"active","content":content,"source":"user","evidence_available":false,
        "trust_tier":"user_asserted","updated_at_ts":1,"expires_at_ts":null,"supersedes_memory_id":null,
        "last_recalled_at_ts":null,"freshness":"fresh"})).unwrap()
}
pub(super) fn export(items: Vec<MemoryListItem>) -> MemoryExport {
    let checksum = digest(&serde_json::to_vec(&items).unwrap());
    MemoryExport {
        schema_version: 1,
        exported_at_ts: 100,
        scope_kind: "principal".into(),
        items,
        checksum,
    }
}
pub(super) fn parse_policy() -> ParsePolicy {
    ParsePolicy {
        upload: policy(64 * 1024),
        value_bytes: 128 * 1024,
    }
}
pub(super) fn upload_raw(db: &Connection, actor: &AuthIdentity, raw: &[u8]) -> UploadStatus {
    let upload = create(
        db,
        actor,
        &CreateUpload {
            file_digest: digest(raw),
            total_bytes: raw.len() as u64,
        },
        policy(64 * 1024),
        100,
    )
    .unwrap();
    let mut offset = 0;
    for chunk in raw.chunks(64 * 1024) {
        append(
            db,
            actor,
            &upload.upload_id,
            offset,
            &digest(chunk),
            chunk,
            101,
        )
        .unwrap();
        offset += chunk.len() as u64;
    }
    upload
}
pub(super) fn preview(
    db: &Connection,
    actor: &AuthIdentity,
    items: Vec<MemoryListItem>,
) -> (UploadStatus, ux::MemoryImportPreview) {
    let raw = serde_json::to_vec(&export(items)).unwrap();
    let upload = upload_raw(db, actor, &raw);
    let preview = parse_and_preview(db, actor, &upload.upload_id, parse_policy(), 102).unwrap();
    (upload, preview)
}
pub(super) fn confirmation(preview: &MemoryImportPreview) -> MemoryImportConfirmRequest {
    MemoryImportConfirmRequest {
        import_id: preview.import_id.clone(),
        expected_payload_digest: preview.payload_digest.clone(),
    }
}
pub(super) fn confirm(
    db: &Connection,
    actor: &AuthIdentity,
    preview: &MemoryImportPreview,
) -> anyhow::Result<ux::MemoryImportResult> {
    checked_write(db, actor, |db| {
        ux::confirm_memory_import(db, &actor.principal_id, &confirmation(preview), 103)
    })
}
pub(super) fn clear(db: &Connection, actor: &AuthIdentity, mode: &str) -> anyhow::Result<()> {
    checked_write(db, actor, |db| {
        let before = ux::clear_preview(db, &actor.principal_id, mode)?;
        ux::clear_with_mode(
            db,
            &actor.principal_id,
            &ux::MemoryBulkClearRequest {
                mode: before.mode,
                expected_transcript_rows: before.transcript_rows,
                expected_derived_rows: before.derived_rows,
            },
            104,
        )?;
        Ok(())
    })
}
pub(super) fn staged_count(db: &Connection, id: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM memory_import_upload_items WHERE upload_id=?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
