#![allow(dead_code)]

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteExecutorAdmission {
    pub(crate) schema_version: u32,
    pub(crate) worker_id: String,
    pub(crate) supported_protocol_versions: Vec<u32>,
    pub(crate) capability_digests: Vec<String>,
    pub(crate) attestation_digest: String,
}

pub(crate) fn validate_feature_config(
    config: &claw_core::config::RemoteExecutorConfig,
) -> Result<()> {
    if !config.enabled {
        bail!("remote_executor_disabled");
    }
    let endpoint = config
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("remote_executor_endpoint_missing"))?;
    if !endpoint.starts_with("https://") {
        bail!("remote_executor_endpoint_requires_tls");
    }
    if config.trusted_attestation_digests.is_empty() {
        bail!("remote_executor_attestation_allowlist_missing");
    }
    for digest in &config.trusted_attestation_digests {
        validate_digest(digest)?;
    }
    Ok(())
}

pub(crate) fn validate_admission(
    config: &claw_core::config::RemoteExecutorConfig,
    admission: &RemoteExecutorAdmission,
) -> Result<()> {
    validate_feature_config(config)?;
    if admission.schema_version != crate::remote_executor_contract::REMOTE_EXECUTOR_SCHEMA_VERSION
        || !admission
            .supported_protocol_versions
            .contains(&crate::remote_executor_contract::REMOTE_EXECUTOR_SCHEMA_VERSION)
    {
        bail!("remote_executor_protocol_version_unsupported");
    }
    if admission.worker_id.trim().is_empty() || admission.capability_digests.is_empty() {
        bail!("remote_executor_admission_incomplete");
    }
    validate_digest(&admission.attestation_digest)?;
    if !config
        .trusted_attestation_digests
        .contains(&admission.attestation_digest)
    {
        bail!("remote_executor_attestation_untrusted");
    }
    for digest in &admission.capability_digests {
        validate_digest(digest)?;
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
    {
        bail!("remote_executor_digest_invalid");
    }
    Ok(())
}

#[cfg(test)]
#[path = "remote_executor_admission_tests.rs"]
mod tests;
