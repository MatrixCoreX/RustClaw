use serde_json::Value;

use super::*;
use crate::memory::ux::{import_items, MemoryImportConfirmRequest, MemoryImportResult};
use staged_payload::Descriptor;

/// Called only inside the existing whole-confirm transaction after the session
/// owner, pending status and expected digest have been checked.
pub(in crate::memory) fn confirm_staged(
    db: &Connection,
    principal: &str,
    user_key: &str,
    request: &MemoryImportConfirmRequest,
    payload: &Value,
    now: i64,
) -> anyhow::Result<MemoryImportResult> {
    anyhow::ensure!(!db.is_autocommit(), UploadError::StateConflict);
    schema::require_schema(db)?;
    let descriptor = Descriptor::decode(payload)?;
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_import_uploads
        WHERE upload_id=?1 AND principal_id=?2 AND state='preview' AND preview_import_id=?3)",
        params![descriptor.upload_id, principal, request.import_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(valid, UploadError::Corrupt);
    anyhow::ensure!(
        staged_payload::payload_digest(db, &descriptor)? == request.expected_payload_digest,
        UploadError::Corrupt
    );
    let mut imported_items = 0;
    let mut existing_items = 0;
    let mut admission = None;
    let mut prospective =
        (descriptor.accepted_items > 0).then(|| super::super::ux::GrowthAdmission::begin(db));
    staged_payload::each_item(db, &descriptor, |item| {
        let changed =
            import_items::insert(db, principal, user_key, &request.import_id, &item, now)?;
        if changed == 1 {
            if let Some(prospective) = prospective.take() {
                admission = Some(prospective?);
            }
            imported_items += 1;
        } else {
            existing_items += 1;
        }
        Ok(())
    })?;
    // Detect in-transaction trigger/corruption effects before canonical commit.
    anyhow::ensure!(
        staged_payload::payload_digest(db, &descriptor)? == request.expected_payload_digest,
        UploadError::Corrupt
    );
    db.execute(
        "UPDATE memory_import_sessions SET status='confirmed',confirmed_at_ts=?1
        WHERE import_id=?2 AND principal_id=?3 AND status='preview'",
        params![now, request.import_id, principal],
    )?;
    mark_confirmed(db, principal, &request.import_id, now)?;
    if let Some(admission) = admission {
        admission.finish(db)?;
    }
    Ok(MemoryImportResult {
        schema_version: 1,
        import_id: request.import_id.clone(),
        status: "confirmed".into(),
        imported_items,
        existing_items,
    })
}

pub(in crate::memory) fn mark_confirmed(
    db: &Connection,
    principal: &str,
    id: &str,
    now: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(!db.is_autocommit(), UploadError::StateConflict);
    schema::require_schema(db)?;
    let confirmed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_import_sessions
         WHERE import_id=?1 AND principal_id=?2 AND status='confirmed')",
        params![id, principal],
        |row| row.get(0),
    )?;
    anyhow::ensure!(confirmed, UploadError::StateConflict);
    let exhausted: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_import_uploads
        WHERE principal_id=?1 AND preview_import_id=?2 AND state='preview' AND revision>=?3)",
        params![principal, id, i64::MAX],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!exhausted, UploadError::RevisionExhausted);
    db.execute("DELETE FROM memory_import_upload_items WHERE upload_id IN
        (SELECT upload_id FROM memory_import_uploads WHERE principal_id=?1 AND preview_import_id=?2 AND state='preview')",
        params![principal,id])?;
    db.execute(
        "UPDATE memory_import_uploads SET state='confirmed',revision=revision+1,updated_at_ts=?1
        WHERE principal_id=?2 AND preview_import_id=?3 AND state='preview'",
        params![now, principal, id],
    )?;
    Ok(())
}
