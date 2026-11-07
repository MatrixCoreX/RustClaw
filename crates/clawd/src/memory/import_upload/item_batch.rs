use std::time::Instant;

use super::*;

/// Byte-bounded staging batches amortize DBSTAT/admission without ever buffering
/// the whole export or opening a write transaction while reading its input.
pub(super) struct ItemBatch {
    rows: Vec<(u64, String)>,
    bytes: u64,
    limit: u64,
    started: Instant,
    start_ts: i64,
}
impl ItemBatch {
    pub(super) fn new(limit: u64, start_ts: i64) -> Self {
        Self {
            rows: Vec::new(),
            bytes: 0,
            limit,
            started: Instant::now(),
            start_ts,
        }
    }
    pub(super) fn push(
        &mut self,
        db: &Connection,
        actor: &AuthIdentity,
        lease: &ParseLease,
        ordinal: u64,
        value_json: String,
        policy: UploadPolicy,
    ) -> anyhow::Result<()> {
        let bytes = value_json.len() as u64;
        if !self.rows.is_empty()
            && self.bytes.checked_add(bytes).ok_or(UploadError::Corrupt)? > self.limit
        {
            self.flush(db, actor, lease, policy)?;
        }
        self.bytes = self.bytes.checked_add(bytes).ok_or(UploadError::Corrupt)?;
        self.rows.push((ordinal, value_json));
        if self.bytes >= self.limit {
            self.flush(db, actor, lease, policy)?;
        }
        Ok(())
    }
    pub(super) fn flush(
        &mut self,
        db: &Connection,
        actor: &AuthIdentity,
        lease: &ParseLease,
        policy: UploadPolicy,
    ) -> anyhow::Result<()> {
        if self.rows.is_empty() {
            return Ok(());
        }
        checked_write(db, actor, |db| {
            lease::require_lease(db, actor, lease)?;
            let admission = super::super::ux::GrowthAdmission::begin(db)?;
            for (ordinal, value) in &self.rows {
                db.execute(
                    "INSERT INTO memory_import_upload_items(upload_id,item_index,item_json)
                    VALUES(?1,?2,?3)",
                    params![
                        lease.upload_id,
                        i64::try_from(*ordinal).map_err(|_| UploadError::Corrupt)?,
                        value
                    ],
                )?;
            }
            let now = self
                .start_ts
                .checked_add(
                    i64::try_from(self.started.elapsed().as_secs())
                        .map_err(|_| UploadError::InvalidMetadata)?,
                )
                .ok_or(UploadError::InvalidMetadata)?;
            renew_parser(db, actor, lease, now, policy)?;
            admission.finish(db)?;
            Ok(())
        })?;
        self.rows.clear();
        self.bytes = 0;
        Ok(())
    }
}
