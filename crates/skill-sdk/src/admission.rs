use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capability_request::RuntimePermissionRequest;
use crate::manifest::{validate_safe_name, validate_sha256, PackageManifest};
use crate::platform::HostPlatform;
use crate::receipt::InstallReceipt;
use crate::{SkillSdkError, SkillSdkResult};

pub const HOST_POLICY_GRANT_SCHEMA_VERSION: u32 = 1;
pub const ADMISSION_RECEIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSource {
    ReleaseBaseline,
    Operator,
    AdminApi,
    PolicyAutomation,
    Migration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionState {
    AwaitingPolicyApproval,
    InstalledDisabled,
    Enabled,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantedCapability {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostPolicyGrant {
    pub schema_version: u32,
    pub skill_name: String,
    pub version: String,
    pub semantic_contract_digest: String,
    pub capabilities: Vec<GrantedCapability>,
    pub permissions: RuntimePermissionRequest,
    pub risk_level: HostRiskLevel,
    #[serde(default)]
    pub auto_invocable: bool,
    pub approval_source: ApprovalSource,
    pub approved_at_unix: u64,
}

impl HostPolicyGrant {
    pub fn validate_against(&self, manifest: &PackageManifest) -> SkillSdkResult<()> {
        if self.schema_version != HOST_POLICY_GRANT_SCHEMA_VERSION {
            return Err(SkillSdkError::new(
                "policy_grant_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        validate_safe_name(&self.skill_name, "policy_grant.skill_name")?;
        validate_sha256(
            &self.semantic_contract_digest,
            "policy_grant.semantic_contract_digest",
        )?;
        if self.skill_name != manifest.package.name || self.version != manifest.package.version {
            return Err(SkillSdkError::new(
                "policy_grant_identity_mismatch",
                format!(
                    "grant={}:{} manifest={}:{}",
                    self.skill_name, self.version, manifest.package.name, manifest.package.version
                ),
            ));
        }
        if self.semantic_contract_digest != manifest.capability_request_digest()? {
            return Err(SkillSdkError::new(
                "policy_grant_semantic_digest_mismatch",
                format!("skill={}", self.skill_name),
            ));
        }
        if self.approved_at_unix == 0 {
            return Err(SkillSdkError::new(
                "policy_grant_approval_missing",
                format!("skill={}", self.skill_name),
            ));
        }
        let request = manifest.effective_capability_request()?;
        validate_permission_subset(&self.permissions, &request.permissions)?;
        let requested = request
            .capabilities
            .iter()
            .map(|capability| (capability.name.as_str(), capability.action.as_deref()))
            .collect::<BTreeSet<_>>();
        let mut granted = BTreeSet::new();
        for capability in &self.capabilities {
            let identity = (capability.name.as_str(), capability.action.as_deref());
            if !requested.contains(&identity) {
                return Err(SkillSdkError::new(
                    "policy_grant_capability_not_requested",
                    format!("name={} action={:?}", capability.name, capability.action),
                ));
            }
            if !granted.insert(identity) {
                return Err(SkillSdkError::new(
                    "policy_grant_capability_duplicate",
                    format!("name={} action={:?}", capability.name, capability.action),
                ));
            }
        }
        if self.capabilities.is_empty() {
            return Err(SkillSdkError::new(
                "policy_grant_capability_missing",
                format!("skill={}", self.skill_name),
            ));
        }
        Ok(())
    }

    pub fn digest(&self, manifest: &PackageManifest) -> SkillSdkResult<String> {
        self.validate_against(manifest)?;
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(self)?)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReceipt {
    pub schema_version: u32,
    pub skill_name: String,
    pub version: String,
    pub package_digest: String,
    pub manifest_digest: String,
    pub artifact_set_digest: String,
    pub install_receipt_digest: String,
    pub semantic_contract_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_policy_digest: Option<String>,
    pub registry_generation: u64,
    pub platform: HostPlatform,
    pub state: AdmissionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_source: Option<ApprovalSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at_unix: Option<u64>,
    pub admitted_at_unix: u64,
}

impl AdmissionReceipt {
    pub fn validate(&self) -> SkillSdkResult<()> {
        if self.schema_version != ADMISSION_RECEIPT_SCHEMA_VERSION {
            return Err(SkillSdkError::new(
                "admission_receipt_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        validate_safe_name(&self.skill_name, "admission_receipt.skill_name")?;
        for (field, digest) in [
            ("package_digest", self.package_digest.as_str()),
            ("manifest_digest", self.manifest_digest.as_str()),
            ("artifact_set_digest", self.artifact_set_digest.as_str()),
            (
                "install_receipt_digest",
                self.install_receipt_digest.as_str(),
            ),
            (
                "semantic_contract_digest",
                self.semantic_contract_digest.as_str(),
            ),
        ] {
            validate_sha256(digest, &format!("admission_receipt.{field}"))?;
        }
        if let Some(digest) = self.granted_policy_digest.as_deref() {
            validate_sha256(digest, "admission_receipt.granted_policy_digest")?;
        }
        if self.version.trim().is_empty()
            || self.registry_generation == 0
            || self.admitted_at_unix == 0
        {
            return Err(SkillSdkError::new(
                "admission_receipt_required_field_missing",
                format!("skill={}", self.skill_name),
            ));
        }
        let approval_fields = [
            self.granted_policy_digest.is_some(),
            self.approval_source.is_some(),
            self.approved_at_unix.is_some(),
        ];
        let approval_absent = approval_fields.iter().all(|present| !present);
        let approval_complete = approval_fields.iter().all(|present| *present)
            && self.approved_at_unix.is_some_and(|value| value > 0);
        if !approval_absent && !approval_complete {
            return Err(SkillSdkError::new(
                "admission_receipt_grant_incomplete",
                format!("skill={}", self.skill_name),
            ));
        }
        match self.state {
            AdmissionState::AwaitingPolicyApproval if !approval_absent => Err(SkillSdkError::new(
                "admission_receipt_state_invalid",
                "awaiting_policy_approval must not carry an active grant",
            )),
            AdmissionState::InstalledDisabled | AdmissionState::Enabled if !approval_complete => {
                Err(SkillSdkError::new(
                    "admission_receipt_grant_missing",
                    format!("state={:?}", self.state),
                ))
            }
            _ => Ok(()),
        }
    }

    pub fn digest(&self) -> SkillSdkResult<String> {
        self.validate()?;
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(self)?)))
    }

    pub fn from_install(
        install: &InstallReceipt,
        manifest: &PackageManifest,
        registry_generation: u64,
        state: AdmissionState,
        grant: Option<&HostPolicyGrant>,
        admitted_at_unix: u64,
    ) -> SkillSdkResult<Self> {
        install.verifies_manifest(manifest)?;
        let (granted_policy_digest, approval_source, approved_at_unix) = match grant {
            Some(grant) => (
                Some(grant.digest(manifest)?),
                Some(grant.approval_source),
                Some(grant.approved_at_unix),
            ),
            None => (None, None, None),
        };
        let receipt = Self {
            schema_version: ADMISSION_RECEIPT_SCHEMA_VERSION,
            skill_name: install.skill_name.clone(),
            version: install.version.clone(),
            package_digest: install.source_digest.clone(),
            manifest_digest: install.manifest_digest.clone(),
            artifact_set_digest: install.artifact_set_digest()?,
            install_receipt_digest: install.digest()?,
            semantic_contract_digest: manifest.capability_request_digest()?,
            granted_policy_digest,
            registry_generation,
            platform: install.platform.clone(),
            state,
            approval_source,
            approved_at_unix,
            admitted_at_unix,
        };
        receipt.validate()?;
        Ok(receipt)
    }
}

fn validate_permission_subset(
    grant: &RuntimePermissionRequest,
    request: &RuntimePermissionRequest,
) -> SkillSdkResult<()> {
    for (name, granted, requested) in [
        ("llm_gateway", grant.llm_gateway, request.llm_gateway),
        ("network", grant.network, request.network),
        (
            "filesystem_read",
            grant.filesystem_read,
            request.filesystem_read,
        ),
        (
            "filesystem_write",
            grant.filesystem_write,
            request.filesystem_write,
        ),
        ("subprocess", grant.subprocess, request.subprocess),
        (
            "package_install",
            grant.package_install,
            request.package_install,
        ),
        (
            "privilege_escalation",
            grant.privilege_escalation,
            request.privilege_escalation,
        ),
        (
            "external_publish",
            grant.external_publish,
            request.external_publish,
        ),
    ] {
        if granted && !requested {
            return Err(SkillSdkError::new(
                "policy_grant_permission_not_requested",
                format!("permission={name}"),
            ));
        }
    }
    let requested_credentials = request.credential_refs.iter().collect::<BTreeSet<_>>();
    let mut granted_credentials = BTreeSet::new();
    for credential in &grant.credential_refs {
        if !requested_credentials.contains(credential) {
            return Err(SkillSdkError::new(
                "policy_grant_permission_not_requested",
                format!("credential_ref={credential}"),
            ));
        }
        if !granted_credentials.insert(credential) {
            return Err(SkillSdkError::new(
                "policy_grant_credential_duplicate",
                format!("credential_ref={credential}"),
            ));
        }
    }
    Ok(())
}
