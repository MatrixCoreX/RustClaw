use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    BadRequest,
    TooLarge,
    NotFound,
    Conflict,
    Unavailable,
    Busy,
    StorageFull,
    Internal,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Failure {
    pub(crate) kind: FailureKind,
    pub(crate) code: &'static str,
    pub(crate) retry_after_seconds: Option<u64>,
}

/// Stable machine errors only; never classify a provider/user-visible sentence.
pub(crate) fn failure(error: &anyhow::Error) -> Option<Failure> {
    use FailureKind::*;
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<UploadError>() {
            let kind = match error {
                UploadError::OuterTransaction => Internal,
                UploadError::FileTooLarge
                | UploadError::ChunkTooLarge
                | UploadError::ValueTooLarge => TooLarge,
                UploadError::NotFound => NotFound,
                UploadError::StateConflict
                | UploadError::OffsetConflict
                | UploadError::Incomplete
                | UploadError::ParserBusy
                | UploadError::LeaseChanged
                | UploadError::RevisionExhausted
                | UploadError::AlreadyConfirmed => Conflict,
                UploadError::SchemaUnavailable
                | UploadError::Corrupt
                | UploadError::IngressUnavailable => Unavailable,
                _ => BadRequest,
            };
            return Some(Failure {
                kind,
                code: error.code(),
                retry_after_seconds: matches!(error, UploadError::ParserBusy).then_some(1),
            });
        }
        if let Some(error) = cause.downcast_ref::<super::super::growth_availability::GrowthError>()
        {
            let kind = match error.0 {
                super::super::growth_availability::GrowthDecision::DomainBudgetReached
                | super::super::growth_availability::GrowthDecision::ExplicitStopUnbound => {
                    StorageFull
                }
                _ => Unavailable,
            };
            return Some(Failure {
                kind,
                code: error.code(),
                retry_after_seconds: None,
            });
        }
        if let Some(error) = cause.downcast_ref::<super::super::storage_budget::ReadError>() {
            return Some(Failure {
                kind: Unavailable,
                code: error.code(),
                retry_after_seconds: None,
            });
        }
        if let Some(error) = cause.downcast_ref::<rusqlite::Error>() {
            use rusqlite::ErrorCode;
            let (kind, code, retry) = match error.sqlite_error_code() {
                Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
                    (Busy, "memory_import_storage_busy", Some(1))
                }
                Some(ErrorCode::DiskFull) => (StorageFull, "memory_import_storage_full", None),
                Some(ErrorCode::ReadOnly) => (Unavailable, "memory_import_storage_readonly", None),
                _ => (Internal, "memory_import_storage_failed", None),
            };
            return Some(Failure {
                kind,
                code,
                retry_after_seconds: retry,
            });
        }
    }
    None
}
