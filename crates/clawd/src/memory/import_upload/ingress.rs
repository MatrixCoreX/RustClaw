use claw_core::config::WebdConfig;

use super::*;

/// A projection of existing webd limits, not a second set of defaults.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IngressPolicy {
    pub(crate) json_bytes: usize,
    pub(crate) upload_bytes: usize,
    pub(crate) body_read_seconds: u64,
    pub(crate) upload_read_seconds: u64,
}
impl IngressPolicy {
    pub(crate) fn from_config(config: &WebdConfig) -> Self {
        Self {
            json_bytes: config.request_limits.max_json_body_bytes,
            upload_bytes: config.max_incoming_body_bytes,
            body_read_seconds: config.request_limits.body_read_timeout_seconds,
            upload_read_seconds: config.request_limits.upload_body_read_timeout_seconds,
        }
    }
    pub(crate) fn chunk_bytes(self) -> anyhow::Result<usize> {
        anyhow::ensure!(
            self.json_bytes > 0
                && self.upload_bytes > 0
                && self.body_read_seconds > 0
                && self.upload_read_seconds > 0,
            UploadError::IngressUnavailable
        );
        Ok(self.json_bytes.min(self.upload_bytes))
    }
    pub(crate) fn parsing(self, lease_seconds: u64) -> anyhow::Result<ParsePolicy> {
        let chunk = self.chunk_bytes()?;
        anyhow::ensure!(
            lease_seconds > 0 && self.json_bytes <= i64::MAX as usize,
            UploadError::IngressUnavailable
        );
        Ok(ParsePolicy {
            upload: UploadPolicy {
                chunk_bytes: chunk as u64,
                parser_lease_seconds: lease_seconds,
            },
            value_bytes: self.json_bytes as u64,
        })
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct TransportPolicy {
    pub(crate) schema_version: u8,
    pub(crate) protocol: &'static str,
    pub(crate) max_file_bytes: String,
    pub(crate) max_chunk_bytes: String,
    pub(crate) max_value_bytes: String,
    pub(crate) file_limit_scope: &'static str,
    pub(crate) growth_rechecked_for_each_chunk_and_staging_batch: bool,
    pub(crate) whole_preview_and_confirmation: bool,
    pub(crate) max_items: Option<u64>,
    pub(crate) max_content_characters: Option<u64>,
}
pub(crate) fn transport_policy(
    db: &Connection,
    actor: &AuthIdentity,
    ingress: IngressPolicy,
    lease_seconds: u64,
) -> anyhow::Result<TransportPolicy> {
    let policy = ingress.parsing(lease_seconds)?;
    checked_read(db, actor, |db| {
        let budget = super::super::storage_budget::Snapshot::load(db)?;
        Ok(TransportPolicy {
            schema_version: 1,
            protocol: "memory-import-upload-v1",
            max_file_bytes: budget.limit.to_string(),
            max_chunk_bytes: policy.upload.chunk_bytes.to_string(),
            max_value_bytes: policy.value_bytes.to_string(),
            file_limit_scope: "memory_domain_budget",
            growth_rechecked_for_each_chunk_and_staging_batch: true,
            whole_preview_and_confirmation: true,
            max_items: None,
            max_content_characters: None,
        })
    })
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod tests;
