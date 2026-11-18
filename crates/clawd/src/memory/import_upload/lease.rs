use super::*;

/// Host-only fencing token, deliberately not a serializable public status.
#[derive(Debug, Clone)]
pub(crate) struct ParseLease {
    pub(super) upload_id: String,
    pub(super) token: String,
    pub(super) file_digest: String,
    pub(super) total_bytes: u64,
    pub(super) chunk_bytes: u64,
}

fn expires(now: i64, seconds: u64) -> anyhow::Result<i64> {
    anyhow::ensure!(seconds > 0 && now >= 0, UploadError::InvalidMetadata);
    let seconds = i64::try_from(seconds).map_err(|_| UploadError::InvalidMetadata)?;
    now.checked_add(seconds)
        .ok_or_else(|| UploadError::InvalidMetadata.into())
}

pub(crate) fn claim_parser(
    db: &Connection,
    actor: &AuthIdentity,
    id: &str,
    now: i64,
    policy: UploadPolicy,
) -> anyhow::Result<ParseLease> {
    let expiry = expires(now, policy.parser_lease_seconds)?;
    checked_write(db, actor, |db| {
        let upload = load(db, actor, id)?;
        anyhow::ensure!(
            matches!(upload.state, UploadState::Uploading | UploadState::Parsing),
            UploadError::StateConflict
        );
        anyhow::ensure!(upload.received == upload.total, UploadError::Incomplete);
        if upload.state == UploadState::Parsing {
            anyhow::ensure!(
                upload.lease_until.is_some_and(|previous| previous <= now),
                UploadError::ParserBusy
            );
        }
        let stored: u64 = db.query_row(
            "SELECT COALESCE(SUM(byte_count),0)
            FROM memory_import_upload_chunks WHERE upload_id=?1",
            [id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(stored == upload.total, UploadError::Corrupt);
        upload.can_advance()?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        // Restarting an abandoned parse can discard only non-canonical staging.
        // Raw chunks stay durable, and an obsolete worker loses its token.
        db.execute(
            "DELETE FROM memory_import_upload_items WHERE upload_id=?1",
            [id],
        )?;
        db.execute("UPDATE memory_import_uploads SET state='parsing',parser_token=?1,parser_lease_until_ts=?2,
            revision=revision+1,updated_at_ts=?3 WHERE upload_id=?4 AND principal_id=?5",
            params![token,expiry,now,id,actor.principal_id])?;
        Ok(ParseLease {
            upload_id: id.into(),
            token,
            file_digest: upload.file_digest,
            total_bytes: upload.total,
            chunk_bytes: upload.chunk,
        })
    })
}

pub(super) fn require_lease(
    db: &Connection,
    actor: &AuthIdentity,
    lease: &ParseLease,
) -> anyhow::Result<StoredUpload> {
    let upload = load(db, actor, &lease.upload_id)?;
    anyhow::ensure!(
        upload.state == UploadState::Parsing
            && upload.parser_token.as_deref() == Some(lease.token.as_str())
            && upload.total == lease.total_bytes
            && upload.chunk == lease.chunk_bytes
            && upload.file_digest == lease.file_digest,
        UploadError::LeaseChanged
    );
    Ok(upload)
}

pub(crate) fn renew_parser(
    db: &Connection,
    actor: &AuthIdentity,
    lease: &ParseLease,
    now: i64,
    policy: UploadPolicy,
) -> anyhow::Result<()> {
    let expiry = expires(now, policy.parser_lease_seconds)?;
    checked_write(db, actor, |db| {
        let upload = require_lease(db, actor, lease)?;
        upload.can_advance()?;
        // Expiry permits another claimant; it is not a wall-clock kill of the
        // original worker. The same token may renew until another claim wins.
        db.execute("UPDATE memory_import_uploads SET parser_lease_until_ts=?1,
            revision=revision+1,updated_at_ts=?2 WHERE upload_id=?3 AND principal_id=?4 AND parser_token=?5",
            params![expiry,now,lease.upload_id,actor.principal_id,lease.token])?;
        Ok(())
    })
}
