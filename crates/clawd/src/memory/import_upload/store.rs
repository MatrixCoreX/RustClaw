use super::*;

pub(crate) fn create(
    db: &Connection,
    actor: &AuthIdentity,
    request: &CreateUpload,
    policy: UploadPolicy,
    now: i64,
) -> anyhow::Result<UploadStatus> {
    anyhow::ensure!(
        valid_digest(&request.file_digest)
            && request.total_bytes > 0
            && policy.chunk_bytes > 0
            && policy.chunk_bytes <= i64::MAX as u64,
        UploadError::InvalidMetadata
    );
    checked_write(db, actor, |db| {
        let existing: Option<String> = db.query_row("SELECT upload_id FROM memory_import_uploads
            WHERE principal_id=?1 AND file_digest=?2 AND state IN ('uploading','parsing','preview')",
            params![actor.principal_id,request.file_digest],|row|row.get(0)).optional()?;
        if let Some(id) = existing {
            let upload = load(db, actor, &id)?;
            anyhow::ensure!(
                upload.total == request.total_bytes,
                UploadError::StateConflict
            );
            return Ok(upload.public());
        }
        let limit = super::super::storage_budget::Snapshot::load(db)?.limit;
        anyhow::ensure!(
            request.total_bytes <= limit && request.total_bytes <= i64::MAX as u64,
            UploadError::FileTooLarge
        );
        let admission = super::super::ux::GrowthAdmission::begin(db)?;
        let id = format!("memory_upload_{}", uuid::Uuid::new_v4().simple());
        db.execute(
            "INSERT INTO memory_import_uploads(upload_id,principal_id,file_digest,total_bytes,
            received_bytes,chunk_bytes,state,revision,created_at_ts,updated_at_ts)
            VALUES(?1,?2,?3,?4,0,?5,'uploading',1,?6,?6)",
            params![
                id,
                actor.principal_id,
                request.file_digest,
                request.total_bytes,
                policy.chunk_bytes,
                now
            ],
        )?;
        admission.finish(db)?;
        Ok(load(db, actor, &id)?.public())
    })
}

pub(crate) fn status(
    db: &Connection,
    actor: &AuthIdentity,
    id: &str,
) -> anyhow::Result<UploadStatus> {
    checked_read(db, actor, |db| Ok(load(db, actor, id)?.public()))
}

pub(crate) fn chunk_budget(db: &Connection, actor: &AuthIdentity, id: &str) -> anyhow::Result<u64> {
    checked_read(db, actor, |db| {
        let upload = load(db, actor, id)?;
        anyhow::ensure!(
            upload.state != UploadState::Cancelled,
            UploadError::StateConflict
        );
        Ok(upload.chunk)
    })
}

pub(crate) fn append(
    db: &Connection,
    actor: &AuthIdentity,
    id: &str,
    offset: u64,
    expected_digest: &str,
    bytes: &[u8],
    now: i64,
) -> anyhow::Result<UploadStatus> {
    anyhow::ensure!(!bytes.is_empty(), UploadError::InvalidMetadata);
    anyhow::ensure!(
        valid_digest(expected_digest) && digest(bytes) == expected_digest,
        UploadError::ChunkDigestMismatch
    );
    checked_write(db, actor, |db| {
        let upload = load(db, actor, id)?;
        anyhow::ensure!(
            upload.state != UploadState::Cancelled,
            UploadError::StateConflict
        );
        anyhow::ensure!(
            bytes.len() as u64 <= upload.chunk,
            UploadError::ChunkTooLarge
        );
        anyhow::ensure!(offset <= upload.total, UploadError::OffsetConflict);
        let existing = db
            .query_row(
                "SELECT content_digest,content,byte_count FROM memory_import_upload_chunks
            WHERE upload_id=?1 AND byte_offset=?2",
                params![id, offset],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<Vec<u8>>>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((previous_digest, previous, count)) = existing {
            anyhow::ensure!(
                previous_digest == expected_digest
                    && count == bytes.len() as u64
                    && previous.as_deref().map_or(
                        matches!(upload.state, UploadState::Preview | UploadState::Confirmed),
                        |previous| previous == bytes
                    ),
                UploadError::OffsetConflict
            );
            return Ok(upload.public());
        }
        anyhow::ensure!(
            upload.state == UploadState::Uploading,
            UploadError::StateConflict
        );
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(UploadError::OffsetConflict)?;
        anyhow::ensure!(
            offset == upload.received && end <= upload.total,
            UploadError::OffsetConflict
        );
        upload.can_advance()?;
        let admission = super::super::ux::GrowthAdmission::begin(db)?;
        db.execute("INSERT INTO memory_import_upload_chunks(upload_id,byte_offset,byte_count,content_digest,content)
            VALUES(?1,?2,?3,?4,?5)",params![id,offset,bytes.len() as u64,expected_digest,bytes])?;
        let changed = db.execute("UPDATE memory_import_uploads SET received_bytes=?1,revision=revision+1,updated_at_ts=?2
            WHERE upload_id=?3 AND principal_id=?4 AND revision=?5 AND state='uploading'",
            params![end,now,id,actor.principal_id,upload.revision])?;
        anyhow::ensure!(changed == 1, UploadError::StateConflict);
        admission.finish(db)?;
        Ok(load(db, actor, id)?.public())
    })
}

pub(crate) fn cancel(
    db: &Connection,
    actor: &AuthIdentity,
    id: &str,
    now: i64,
) -> anyhow::Result<UploadStatus> {
    checked_write(db, actor, |db| {
        let upload = load(db, actor, id)?;
        if upload.state == UploadState::Cancelled {
            return Ok(upload.public());
        }
        if upload.state == UploadState::Preview {
            super::lifecycle::cancel_preview(db, actor, &upload, now)?;
            return Ok(load(db, actor, id)?.public());
        }
        anyhow::ensure!(
            matches!(upload.state, UploadState::Uploading | UploadState::Parsing),
            UploadError::StateConflict
        );
        upload.can_advance()?;
        db.execute(
            "DELETE FROM memory_import_upload_chunks WHERE upload_id=?1",
            [id],
        )?;
        db.execute(
            "DELETE FROM memory_import_upload_items WHERE upload_id=?1",
            [id],
        )?;
        db.execute("UPDATE memory_import_uploads SET state='cancelled',parser_token=NULL,parser_lease_until_ts=NULL,
            revision=revision+1,updated_at_ts=?1 WHERE upload_id=?2 AND principal_id=?3",
            params![now,id,actor.principal_id])?;
        Ok(load(db, actor, id)?.public())
    })
}
