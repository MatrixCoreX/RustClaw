#![allow(dead_code)]

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub(crate) const REMOTE_EXECUTOR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteArtifactRef {
    pub digest: String,
    pub size_bytes: u64,
    pub chunk_size_bytes: u64,
    pub chunk_digests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteCredentialRef {
    pub reference: String,
    pub audience: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteExecutorLease {
    pub lease_id: String,
    pub owner_id: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub heartbeat_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteExecutorAssignment {
    pub schema_version: u32,
    pub assignment_id: String,
    pub task_id: String,
    pub idempotency_key: String,
    pub code_revision: String,
    pub registry_generation: String,
    pub policy_digest: String,
    pub capability_digest: String,
    pub skill_receipt_digest: String,
    pub workspace_snapshot: Option<RemoteArtifactRef>,
    pub granted_capabilities: Vec<String>,
    pub credential_refs: Vec<RemoteCredentialRef>,
    pub lease: RemoteExecutorLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RemoteExecutorState {
    Accepted,
    Running,
    Waiting,
    Ambiguous,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteExecutorEvent {
    pub schema_version: u32,
    pub assignment_id: String,
    pub lease_id: String,
    pub sequence: u64,
    pub state: RemoteExecutorState,
    pub progress_digest: String,
    pub heartbeat_at_unix: u64,
    pub artifact_refs: Vec<RemoteArtifactRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteTerminalReceipt {
    pub schema_version: u32,
    pub assignment_id: String,
    pub lease_id: String,
    pub terminal_state: RemoteExecutorState,
    pub result_digest: String,
    pub mutation_receipt_digests: Vec<String>,
    pub artifact_refs: Vec<RemoteArtifactRef>,
    pub completed_at_unix: u64,
    pub worker_attestation_digest: String,
}

pub(crate) fn validate_assignment(value: &RemoteExecutorAssignment, now_unix: u64) -> Result<()> {
    validate_schema(value.schema_version)?;
    for (field, raw) in [
        ("assignment_id", value.assignment_id.as_str()),
        ("task_id", value.task_id.as_str()),
        ("idempotency_key", value.idempotency_key.as_str()),
        ("code_revision", value.code_revision.as_str()),
        ("registry_generation", value.registry_generation.as_str()),
        ("lease_id", value.lease.lease_id.as_str()),
        ("lease_owner_id", value.lease.owner_id.as_str()),
    ] {
        validate_identifier(field, raw)?;
    }
    for (field, digest) in [
        ("policy_digest", value.policy_digest.as_str()),
        ("capability_digest", value.capability_digest.as_str()),
        ("skill_receipt_digest", value.skill_receipt_digest.as_str()),
    ] {
        validate_digest(field, digest)?;
    }
    if value.lease.issued_at_unix > now_unix
        || value.lease.expires_at_unix <= now_unix
        || value.lease.expires_at_unix <= value.lease.issued_at_unix
    {
        bail!("remote_executor_lease_invalid");
    }
    if value.granted_capabilities.is_empty() {
        bail!("remote_executor_capability_grant_missing");
    }
    for capability in &value.granted_capabilities {
        validate_identifier("granted_capability", capability)?;
    }
    for credential in &value.credential_refs {
        validate_identifier("credential_reference", &credential.reference)?;
        validate_identifier("credential_audience", &credential.audience)?;
        if credential.expires_at_unix <= now_unix
            || credential.expires_at_unix > value.lease.expires_at_unix
        {
            bail!("remote_executor_credential_lease_invalid");
        }
    }
    if let Some(snapshot) = &value.workspace_snapshot {
        validate_artifact(snapshot)?;
    }
    Ok(())
}

pub(crate) fn validate_event(
    assignment: &RemoteExecutorAssignment,
    event: &RemoteExecutorEvent,
) -> Result<()> {
    validate_schema(event.schema_version)?;
    validate_binding(
        assignment,
        &event.assignment_id,
        &event.lease_id,
        event.sequence,
    )?;
    validate_digest("progress_digest", &event.progress_digest)?;
    for artifact in &event.artifact_refs {
        validate_artifact(artifact)?;
    }
    Ok(())
}

pub(crate) fn validate_terminal_receipt(
    assignment: &RemoteExecutorAssignment,
    receipt: &RemoteTerminalReceipt,
) -> Result<()> {
    validate_schema(receipt.schema_version)?;
    validate_binding(
        assignment,
        &receipt.assignment_id,
        &receipt.lease_id,
        assignment.lease.heartbeat_seq,
    )?;
    if !matches!(
        receipt.terminal_state,
        RemoteExecutorState::Succeeded
            | RemoteExecutorState::Failed
            | RemoteExecutorState::Cancelled
    ) {
        bail!("remote_executor_receipt_not_terminal");
    }
    validate_digest("result_digest", &receipt.result_digest)?;
    validate_digest(
        "worker_attestation_digest",
        &receipt.worker_attestation_digest,
    )?;
    for digest in &receipt.mutation_receipt_digests {
        validate_digest("mutation_receipt_digest", digest)?;
    }
    for artifact in &receipt.artifact_refs {
        validate_artifact(artifact)?;
    }
    Ok(())
}

/// A lost transport cannot prove that an external mutation did not run. The
/// control plane must query/reconcile before considering reassignment.
pub(crate) fn state_after_transport_loss(had_external_effect: bool) -> RemoteExecutorState {
    if had_external_effect {
        RemoteExecutorState::Ambiguous
    } else {
        RemoteExecutorState::Waiting
    }
}

fn validate_binding(
    assignment: &RemoteExecutorAssignment,
    assignment_id: &str,
    lease_id: &str,
    sequence: u64,
) -> Result<()> {
    if assignment.assignment_id != assignment_id || assignment.lease.lease_id != lease_id {
        bail!("remote_executor_binding_mismatch");
    }
    if sequence < assignment.lease.heartbeat_seq {
        bail!("remote_executor_sequence_regression");
    }
    Ok(())
}

fn validate_artifact(artifact: &RemoteArtifactRef) -> Result<()> {
    validate_digest("artifact_digest", &artifact.digest)?;
    if artifact.size_bytes == 0 || artifact.chunk_size_bytes == 0 {
        bail!("remote_executor_artifact_size_invalid");
    }
    if artifact.chunk_digests.is_empty() {
        bail!("remote_executor_artifact_chunks_missing");
    }
    for digest in &artifact.chunk_digests {
        validate_digest("artifact_chunk_digest", digest)?;
    }
    Ok(())
}

fn validate_schema(schema_version: u32) -> Result<()> {
    if schema_version != REMOTE_EXECUTOR_SCHEMA_VERSION {
        bail!("remote_executor_schema_unsupported");
    }
    Ok(())
}

fn validate_digest(field: &str, raw: &str) -> Result<()> {
    if raw.len() != 64
        || !raw
            .chars()
            .all(|value| value.is_ascii_digit() || ('a'..='f').contains(&value))
    {
        bail!("remote_executor_digest_invalid:{field}");
    }
    Ok(())
}

fn validate_identifier(field: &str, raw: &str) -> Result<()> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|ch| ch.is_control() || ch == '/' || ch == '\\')
    {
        bail!("remote_executor_identifier_invalid:{field}");
    }
    Ok(())
}

#[cfg(test)]
#[path = "remote_executor_contract_tests.rs"]
mod tests;
