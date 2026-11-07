use std::time::Instant;

use serde_json::Value;

use super::*;
use crate::memory::ux::{import_items, MemoryImportPreview};
use staged_payload::{Descriptor, STORAGE};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParsePolicy {
    pub(crate) upload: UploadPolicy,
    pub(crate) value_bytes: u64,
}

pub(crate) fn parse_and_preview(
    db: &Connection,
    actor: &AuthIdentity,
    id: &str,
    policy: ParsePolicy,
    now: i64,
) -> anyhow::Result<MemoryImportPreview> {
    let previous = checked_read(db, actor, |db| {
        let upload = load(db, actor, id)?;
        if upload.state == UploadState::Preview {
            return Ok(Some(load_preview(
                db,
                actor,
                upload.preview.as_deref().ok_or(UploadError::Corrupt)?,
            )?));
        }
        anyhow::ensure!(
            upload.state != UploadState::Confirmed,
            UploadError::AlreadyConfirmed
        );
        Ok(None)
    })?;
    if let Some(previous) = previous {
        return Ok(previous);
    }
    let lease = claim_parser(db, actor, id, now, policy.upload)?;
    let started = Instant::now();
    let mut last_renewal = 0;
    let mut descriptor = Descriptor {
        schema_version: 2,
        storage: STORAGE.into(),
        upload_id: id.into(),
        accepted_items: 0,
        skipped_items: 0,
        duplicate_items: 0,
        max_item_bytes: 0,
        has_recent: false,
    };
    let mut ordinal = 0u64;
    let mut batch = item_batch::ItemBatch::new(policy.value_bytes, now);
    let parsed = (|| {
        let mut reader = ChunkReader::new(db, actor, &lease)?;
        let summary = parse_export(&mut reader, policy.value_bytes, |item| {
            let elapsed = started.elapsed().as_secs();
            if elapsed.saturating_sub(last_renewal)
                >= (policy.upload.parser_lease_seconds / 2).max(1)
            {
                let current = now
                    .checked_add(i64::try_from(elapsed).map_err(|_| UploadError::InvalidMetadata)?)
                    .ok_or(UploadError::InvalidMetadata)?;
                renew_parser(db, actor, &lease, current, policy.upload)?;
                last_renewal = elapsed;
            }
            let decision = checked_read(db, actor, |db| {
                lease::require_lease(db, actor, &lease)?;
                import_items::prepare(db, &actor.principal_id, &item, None)
            })?;
            match decision {
                import_items::Decision::Skipped => increment(&mut descriptor.skipped_items)?,
                import_items::Decision::Duplicate => increment(&mut descriptor.duplicate_items)?,
                import_items::Decision::Accepted(value) => {
                    let value_json = serde_json::to_string(&value)?;
                    let item_bytes = value_json.len() as u64;
                    batch.push(db, actor, &lease, ordinal, value_json, policy.upload)?;
                    increment(&mut descriptor.accepted_items)?;
                    descriptor.max_item_bytes = descriptor.max_item_bytes.max(item_bytes);
                    descriptor.has_recent |= value["kind"].as_str() == Some("recent");
                }
            }
            ordinal = ordinal.checked_add(1).ok_or(UploadError::Corrupt)?;
            Ok(())
        })?;
        reader.verify_complete()?;
        batch.flush(db, actor, &lease, policy.upload)?;
        anyhow::ensure!(summary.item_count == ordinal, UploadError::Corrupt);
        publish(db, actor, &lease, &descriptor, now)
    })();
    if parsed.is_err() {
        // A normal failure releases only this parser's provisional work. A
        // crash leaves the durable lease/raw chunks for a later takeover.
        let _ = checked_write(db, actor, |db| {
            let upload = lease::require_lease(db, actor, &lease)?;
            upload.can_advance()?;
            db.execute(
                "DELETE FROM memory_import_upload_items WHERE upload_id=?1",
                [id],
            )?;
            db.execute(
                "UPDATE memory_import_uploads SET state='uploading',parser_token=NULL,
                parser_lease_until_ts=NULL,revision=revision+1,updated_at_ts=?1 WHERE upload_id=?2",
                params![now, id],
            )?;
            Ok(())
        });
    }
    parsed
}

fn increment(value: &mut usize) -> anyhow::Result<()> {
    *value = value.checked_add(1).ok_or(UploadError::Corrupt)?;
    Ok(())
}

fn publish(
    db: &Connection,
    actor: &AuthIdentity,
    lease: &ParseLease,
    descriptor: &Descriptor,
    now: i64,
) -> anyhow::Result<MemoryImportPreview> {
    checked_write(db, actor, |db| {
        let upload = lease::require_lease(db, actor, lease)?;
        upload.can_advance()?;
        let digest = staged_payload::payload_digest(db, descriptor)?;
        let previous = db
            .query_row(
                "SELECT import_id,status FROM memory_import_sessions
            WHERE principal_id=?1 AND payload_digest=?2",
                params![actor.principal_id, digest],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let admission = if descriptor.accepted_items == 0 {
            None
        } else {
            Some(super::super::ux::GrowthAdmission::begin(db)?)
        };
        let import_id = if let Some((id, state)) = previous {
            anyhow::ensure!(state != "confirmed", UploadError::AlreadyConfirmed);
            anyhow::ensure!(state == "preview", UploadError::StateConflict);
            load_preview(db, actor, &id)?;
            id
        } else {
            let id = format!("memory_import_{}", uuid::Uuid::new_v4().simple());
            db.execute("INSERT INTO memory_import_sessions(import_id,principal_id,scope_kind,scope_ref,
                payload_digest,preview_json,status,created_at_ts) VALUES(?1,?2,'principal',?2,?3,?4,'preview',?5)",
                params![id,actor.principal_id,digest,serde_json::to_string(descriptor)?,now])?;
            id
        };
        db.execute(
            "UPDATE memory_import_upload_chunks SET content=NULL WHERE upload_id=?1",
            [&lease.upload_id],
        )?;
        db.execute("UPDATE memory_import_uploads SET state='preview',parser_token=NULL,parser_lease_until_ts=NULL,
            preview_import_id=?1,revision=revision+1,updated_at_ts=?2 WHERE upload_id=?3 AND principal_id=?4",
            params![import_id,now,lease.upload_id,actor.principal_id])?;
        if let Some(admission) = admission {
            admission.finish(db)?;
        }
        Ok(descriptor.preview(import_id, digest))
    })
}

fn load_preview(
    db: &Connection,
    actor: &AuthIdentity,
    id: &str,
) -> anyhow::Result<MemoryImportPreview> {
    let (digest, json, state) = db
        .query_row(
            "SELECT payload_digest,preview_json,status FROM memory_import_sessions
        WHERE import_id=?1 AND principal_id=?2",
            params![id, actor.principal_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(UploadError::Corrupt)?;
    anyhow::ensure!(state == "preview", UploadError::StateConflict);
    let payload: Value = serde_json::from_str(&json).map_err(|_| UploadError::Corrupt)?;
    if staged_payload::is_staged(&payload) {
        let descriptor = Descriptor::decode(&payload)?;
        let owned: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_import_uploads
             WHERE upload_id=?1 AND principal_id=?2 AND state='preview' AND preview_import_id=?3)",
            params![descriptor.upload_id, actor.principal_id, id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(owned, UploadError::Corrupt);
        anyhow::ensure!(
            staged_payload::payload_digest(db, &descriptor)? == digest,
            UploadError::Corrupt
        );
        Ok(descriptor.preview(id.into(), digest))
    } else {
        anyhow::ensure!(
            format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&payload)?)) == digest,
            UploadError::Corrupt
        );
        Ok(MemoryImportPreview {
            schema_version: 1,
            import_id: id.into(),
            payload_digest: digest,
            accepted_items: payload["items"]
                .as_array()
                .ok_or(UploadError::Corrupt)?
                .len(),
            skipped_items: usize::try_from(
                payload["skipped_items"]
                    .as_u64()
                    .ok_or(UploadError::Corrupt)?,
            )
            .map_err(|_| UploadError::Corrupt)?,
            duplicate_items: usize::try_from(
                payload["duplicate_items"]
                    .as_u64()
                    .ok_or(UploadError::Corrupt)?,
            )
            .map_err(|_| UploadError::Corrupt)?,
            trust_tier: "imported_legacy".into(),
            scope_kind: "principal".into(),
        })
    }
}
