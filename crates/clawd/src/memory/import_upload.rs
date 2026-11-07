//! Durable import transport primitives. Upload/parse leases never authorize a
//! canonical import: publication still requires one validated preview/confirm.
use claw_core::types::AuthIdentity;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod lease;
mod item_batch;
mod lifecycle;
mod confirmation;
mod parser;
mod preview;
mod reader;
mod staged_payload;
mod schema;
mod store;

pub(super) use confirmation::{confirm_staged, mark_confirmed};
pub(crate) use lease::{claim_parser, renew_parser, ParseLease};
pub(super) use lifecycle::{purge_for_clear, recent_payload};
pub(crate) use parser::{parse_export, ParsedExport};
pub(crate) use preview::{parse_and_preview, ParsePolicy};
pub(crate) use reader::ChunkReader;
pub(crate) use schema::initialize_schema;
pub(super) use staged_payload::is_staged;
pub(crate) use store::{append, cancel, create, status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadError {
    InvalidMetadata,
    FileTooLarge,
    ChunkTooLarge,
    ChunkDigestMismatch,
    FileDigestMismatch,
    NotFound,
    StateConflict,
    OffsetConflict,
    Incomplete,
    ParserBusy,
    LeaseChanged,
    RevisionExhausted,
    SchemaUnavailable,
    Corrupt,
    ValueTooLarge,
    InvalidJson,
    ExportSchemaUnsupported,
    ExportChecksumMismatch,
    AlreadyConfirmed,
}

impl UploadError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InvalidMetadata => "memory_import_upload_metadata_invalid",
            Self::FileTooLarge => "memory_import_upload_file_too_large",
            Self::ChunkTooLarge => "memory_import_upload_chunk_too_large",
            Self::ChunkDigestMismatch => "memory_import_upload_chunk_digest_mismatch",
            Self::FileDigestMismatch => "memory_import_upload_file_digest_mismatch",
            Self::NotFound => "memory_import_upload_not_found",
            Self::StateConflict => "memory_import_upload_state_conflict",
            Self::OffsetConflict => "memory_import_upload_offset_conflict",
            Self::Incomplete => "memory_import_upload_incomplete",
            Self::ParserBusy => "memory_import_upload_parser_busy",
            Self::LeaseChanged => "memory_import_upload_lease_changed",
            Self::RevisionExhausted => "memory_import_upload_revision_exhausted",
            Self::SchemaUnavailable => "memory_import_upload_schema_unavailable",
            Self::Corrupt => "memory_import_upload_corrupt",
            Self::ValueTooLarge => "memory_import_upload_value_too_large",
            Self::InvalidJson => "memory_import_upload_json_invalid",
            Self::ExportSchemaUnsupported => "memory_import_schema_unsupported",
            Self::ExportChecksumMismatch => "memory_import_checksum_invalid",
            Self::AlreadyConfirmed => "memory_import_already_confirmed",
        }
    }
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(self.code())
    }
}
impl std::error::Error for UploadError {}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UploadState {
    Uploading,
    Parsing,
    Preview,
    Confirmed,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateUpload {
    pub(crate) file_digest: String,
    pub(crate) total_bytes: u64,
}

/// Caller supplies the existing host ingress policy; no second default or
/// model-context-derived upload limit lives in this module.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UploadPolicy {
    pub(crate) chunk_bytes: u64,
    pub(crate) parser_lease_seconds: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct UploadStatus {
    pub(crate) schema_version: u8,
    pub(crate) upload_id: String,
    pub(crate) file_digest: String,
    pub(crate) total_bytes: String,
    pub(crate) received_bytes: String,
    pub(crate) chunk_bytes: String,
    pub(crate) revision: String,
    pub(crate) state: UploadState,
    pub(crate) parser_lease_until_ts: Option<i64>,
    pub(crate) preview_import_id: Option<String>,
}

struct StoredUpload {
    id: String,
    file_digest: String,
    total: u64,
    received: u64,
    chunk: u64,
    revision: u64,
    state: UploadState,
    parser_token: Option<String>,
    lease_until: Option<i64>,
    preview: Option<String>,
}

impl StoredUpload {
    fn public(&self) -> UploadStatus {
        UploadStatus {
            schema_version: 1,
            upload_id: self.id.clone(),
            file_digest: self.file_digest.clone(),
            total_bytes: self.total.to_string(),
            received_bytes: self.received.to_string(),
            chunk_bytes: self.chunk.to_string(),
            revision: self.revision.to_string(),
            state: self.state,
            parser_lease_until_ts: self.lease_until,
            preview_import_id: self.preview.clone(),
        }
    }
    fn can_advance(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.revision < i64::MAX as u64,
            UploadError::RevisionExhausted
        );
        Ok(())
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn load(db: &Connection, actor: &AuthIdentity, id: &str) -> anyhow::Result<StoredUpload> {
    let upload = db
        .query_row(
            "SELECT file_digest,total_bytes,received_bytes,chunk_bytes,revision,state,
                parser_token,parser_lease_until_ts,preview_import_id
         FROM memory_import_uploads WHERE upload_id=?1 AND principal_id=?2",
            params![id, actor.principal_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?
        .ok_or(UploadError::NotFound)?;
    let state = match upload.5.as_str() {
        "uploading" => UploadState::Uploading,
        "parsing" => UploadState::Parsing,
        "preview" => UploadState::Preview,
        "confirmed" => UploadState::Confirmed,
        "cancelled" => UploadState::Cancelled,
        _ => return Err(UploadError::Corrupt.into()),
    };
    anyhow::ensure!(
        valid_digest(&upload.0)
            && upload.1 > 0
            && upload.2 <= upload.1
            && upload.3 > 0
            && upload.3 <= i64::MAX as u64
            && upload.4 > 0
            && (if state == UploadState::Parsing {
                upload.6.is_some() && upload.7.is_some()
            } else {
                upload.6.is_none() && upload.7.is_none()
            }),
        UploadError::Corrupt
    );
    Ok(StoredUpload {
        id: id.into(),
        file_digest: upload.0,
        total: upload.1,
        received: upload.2,
        chunk: upload.3,
        revision: upload.4,
        state,
        parser_token: upload.6,
        lease_until: upload.7,
        preview: upload.8,
    })
}

fn checked_write<T>(
    db: &Connection,
    actor: &AuthIdentity,
    operation: impl FnOnce(&Connection) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    super::ux::atomic_mutation(db, |db| {
        schema::require_schema(db)?;
        crate::repo::auth::require_request_identity(db, actor)?;
        let result = operation(db)?;
        crate::repo::auth::require_request_identity(db, actor)?;
        Ok(result)
    })
}

fn checked_read<T>(
    db: &Connection,
    actor: &AuthIdentity,
    operation: impl FnOnce(&Connection) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let transaction = db
        .is_autocommit()
        .then(|| Transaction::new_unchecked(db, TransactionBehavior::Deferred))
        .transpose()?;
    schema::require_schema(db)?;
    crate::repo::auth::require_request_identity(db, actor)?;
    let result = operation(db)?;
    // Dropping the read snapshot rolls back, never commits accidental writes.
    drop(transaction);
    Ok(result)
}

#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod test_fixtures;
