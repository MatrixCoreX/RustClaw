use serde_json::{json, Value};

use super::*;

pub(in crate::memory) fn recent_payload(
    db: &Connection,
    principal: &str,
    payload: &Value,
    expected_digest: &str,
) -> anyhow::Result<Option<bool>> {
    if !staged_payload::is_staged(payload) {
        return Ok(None);
    }
    let descriptor = staged_payload::Descriptor::decode(payload)?;
    let state = db
        .query_row(
            "SELECT state FROM memory_import_uploads
        WHERE upload_id=?1 AND principal_id=?2",
            params![descriptor.upload_id, principal],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(UploadError::Corrupt)?;
    if state == "preview" {
        // Confirmed receipts have already discarded plaintext staging. Pending
        // ones must verify this privacy classification against their real items.
        anyhow::ensure!(
            staged_payload::payload_digest(db, &descriptor)? == expected_digest,
            UploadError::Corrupt
        );
    } else {
        anyhow::ensure!(state == "confirmed", UploadError::Corrupt);
    }
    Ok(Some(descriptor.has_recent))
}

pub(super) fn cancel_preview(
    db: &Connection,
    actor: &AuthIdentity,
    upload: &StoredUpload,
    now: i64,
) -> anyhow::Result<()> {
    let id = upload.preview.as_deref().ok_or(UploadError::Corrupt)?;
    let (original, state) = db
        .query_row(
            "SELECT payload_digest,status FROM memory_import_sessions
        WHERE import_id=?1 AND principal_id=?2",
            params![id, actor.principal_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or(UploadError::Corrupt)?;
    anyhow::ensure!(state == "preview", UploadError::StateConflict);
    let tombstone =
        json!({"schema_version":1,"purged":true,"import_id":id,"original_payload_digest":original})
            .to_string();
    db.execute(
        "UPDATE memory_import_sessions SET status='cancelled',preview_json=?1,payload_digest=?2
        WHERE import_id=?3 AND principal_id=?4",
        params![
            tombstone,
            digest(tombstone.as_bytes()),
            id,
            actor.principal_id
        ],
    )?;
    let mut after = String::new();
    loop {
        let next = db
            .query_row(
                "SELECT upload_id FROM memory_import_uploads
            WHERE principal_id=?1 AND preview_import_id=?2 AND state='preview' AND upload_id>?3
            ORDER BY upload_id LIMIT 1",
                params![actor.principal_id, id, after],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        let Some(id) = next else { break };
        after = id.clone();
        purge_one(db, &actor.principal_id, &id, now, false)?;
    }
    Ok(())
}

fn purge_one(
    db: &Connection,
    principal: &str,
    id: &str,
    now: i64,
    preserve_confirmed: bool,
) -> anyhow::Result<()> {
    let revision: i64 = db.query_row(
        "SELECT revision FROM memory_import_uploads
        WHERE upload_id=?1 AND principal_id=?2",
        params![id, principal],
        |r| r.get(0),
    )?;
    anyhow::ensure!(revision < i64::MAX, UploadError::RevisionExhausted);
    db.execute(
        "DELETE FROM memory_import_upload_items WHERE upload_id=?1",
        [id],
    )?;
    db.execute(
        "DELETE FROM memory_import_upload_chunks WHERE upload_id=?1",
        [id],
    )?;
    db.execute(
        "UPDATE memory_import_uploads SET
        state=CASE WHEN ?1 AND state='confirmed' THEN state ELSE 'cancelled' END,
        parser_token=NULL,parser_lease_until_ts=NULL,revision=revision+1,updated_at_ts=?2
        WHERE upload_id=?3 AND principal_id=?4",
        params![preserve_confirmed, now, id, principal],
    )?;
    Ok(())
}

/// Invoked only by the existing authenticated whole-clear transaction, after
/// session tombstones are published. There is never a read-time schema repair.
pub(in crate::memory) fn purge_for_clear(
    db: &Connection,
    principal: &str,
    include_derived: bool,
    now: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(!db.is_autocommit(), UploadError::StateConflict);
    schema::require_schema(db)?;
    let mut after = String::new();
    loop {
        let next = db
            .query_row(
                "SELECT upload_id,state,preview_import_id FROM memory_import_uploads
            WHERE principal_id=?1 AND upload_id>?2 ORDER BY upload_id LIMIT 1",
                params![principal, after],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, state, preview)) = next else {
            break;
        };
        after = id.clone();
        // Unparsed raw uploads can contain transcript data; cancelling them on
        // explicit clear avoids retaining an opaque private copy.
        let mut purge = include_derived || matches!(state.as_str(), "uploading" | "parsing");
        if !purge {
            if let Some(preview) = preview {
                let payload = db
                    .query_row(
                        "SELECT preview_json FROM memory_import_sessions
                    WHERE import_id=?1 AND principal_id=?2",
                        params![preview, principal],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
                    .ok_or(UploadError::Corrupt)?;
                let payload: Value =
                    serde_json::from_str(&payload).map_err(|_| UploadError::Corrupt)?;
                purge = payload["purged"].as_bool() == Some(true);
            }
        }
        if purge {
            purge_one(db, principal, &id, now, true)?;
        }
    }
    Ok(())
}
