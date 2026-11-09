//! Public-safe machine evidence for local media I/O failures, never user-visible prose.
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, io};

const PREFIX: &str = "__CHANNEL_MEDIA_IO_ERROR_V1__:";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaIoOperation {
    Inspect,
    Open,
    Read,
    Encrypt,
    Validate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaIoFailureKind {
    QuotaExceeded,
    StorageFull,
    PermissionDenied,
    Unavailable,
    Changed,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelMediaIoError {
    schema_version: u16,
    pub operation: MediaIoOperation,
    pub kind: MediaIoFailureKind,
    pub raw_os_error: Option<i32>,
}

impl ChannelMediaIoError {
    pub fn from_io(operation: MediaIoOperation, error: &io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::QuotaExceeded => MediaIoFailureKind::QuotaExceeded,
            io::ErrorKind::StorageFull => MediaIoFailureKind::StorageFull,
            io::ErrorKind::PermissionDenied => MediaIoFailureKind::PermissionDenied,
            io::ErrorKind::NotFound => MediaIoFailureKind::Unavailable,
            io::ErrorKind::UnexpectedEof => MediaIoFailureKind::Changed,
            _ => MediaIoFailureKind::Other,
        };
        Self {
            schema_version: 1,
            operation,
            kind,
            raw_os_error: error.raw_os_error(),
        }
    }

    pub fn input_changed() -> Self {
        Self {
            schema_version: 1,
            operation: MediaIoOperation::Validate,
            kind: MediaIoFailureKind::Changed,
            raw_os_error: None,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self.kind {
            MediaIoFailureKind::QuotaExceeded => "channel.media.local_quota_exceeded",
            MediaIoFailureKind::StorageFull => "channel.media.local_storage_full",
            MediaIoFailureKind::PermissionDenied => "channel.media.local_permission_denied",
            MediaIoFailureKind::Unavailable => "channel.media.local_unavailable",
            MediaIoFailureKind::Changed => "channel.media.local_input_changed",
            MediaIoFailureKind::Other => "channel.media.local_io_failed",
        }
    }

    pub fn message_key(&self) -> &'static str {
        "channel.error.delivery_failed"
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let value: Self = serde_json::from_str(raw.strip_prefix(PREFIX)?).ok()?;
        (value.schema_version == 1).then_some(value)
    }

    pub fn find_in_chain(error: &(dyn Error + 'static)) -> Option<Self> {
        let mut current = Some(error);
        while let Some(error) = current {
            if let Some(local) = error.downcast_ref::<Self>() {
                return Some(local.clone());
            }
            if let Some(inner) = error
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
            {
                current = Some(inner);
                continue;
            }
            current = error.source();
        }
        None
    }
}

impl fmt::Display for ChannelMediaIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PREFIX)?;
        f.write_str(&serde_json::to_string(self).map_err(|_| fmt::Error)?)
    }
}
impl Error for ChannelMediaIoError {}

#[cfg(test)]
#[path = "channel_media_io_tests.rs"]
mod tests;
