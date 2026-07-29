use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::manifest::validate_relative_path;
use crate::{SkillSdkError, SkillSdkResult};

pub const CAPABILITY_REQUEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedEffect {
    Observe,
    Mutate,
    Validate,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedExecutionMode {
    SyncShort,
    AsyncPreferred,
    AsyncRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSemanticRole {
    InputFile,
    InputDirectory,
    OutputFile,
    OutputDirectory,
    WorkspaceRoot,
    CredentialReference,
    Continuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKindRequest {
    File,
    Directory,
    Image,
    Audio,
    Video,
    StructuredData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigEntryPointKind {
    File,
    Environment,
    Credential,
    PrivateStorage,
    Api,
    LoginState,
    Dependency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityActionRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub effect: RequestedEffect,
    pub execution_mode: RequestedExecutionMode,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub input_roles: BTreeMap<String, InputSemanticRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RuntimePermissionRequest {
    #[serde(default)]
    pub llm_gateway: bool,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub filesystem_read: bool,
    #[serde(default)]
    pub filesystem_write: bool,
    #[serde(default)]
    pub subprocess: bool,
    #[serde(default)]
    pub package_install: bool,
    #[serde(default)]
    pub privilege_escalation: bool,
    #[serde(default)]
    pub external_publish: bool,
    #[serde(default)]
    pub credential_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactContractRequest {
    #[serde(default)]
    pub kinds: Vec<ArtifactKindRequest>,
    #[serde(default)]
    pub output_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceContractRequest {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub selectors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigEntryPointRequest {
    pub kind: ConfigEntryPointKind,
    pub reference: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequestSet {
    pub schema_version: u32,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub permissions: RuntimePermissionRequest,
    pub artifact_contract: ArtifactContractRequest,
    pub evidence_contract: EvidenceContractRequest,
    #[serde(default)]
    pub config_entry_points: Vec<ConfigEntryPointRequest>,
    pub capabilities: Vec<CapabilityActionRequest>,
}

impl CapabilityRequestSet {
    pub fn validate(&self) -> SkillSdkResult<()> {
        if self.schema_version != CAPABILITY_REQUEST_SCHEMA_VERSION {
            return Err(SkillSdkError::new(
                "capability_request_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        validate_json_schema(&self.input_schema, "capability_request.input_schema")?;
        validate_json_schema(&self.output_schema, "capability_request.output_schema")?;
        if self.capabilities.is_empty() {
            return Err(SkillSdkError::new(
                "capability_request_empty",
                "at least one capability action is required",
            ));
        }
        let mut identities = BTreeSet::new();
        for capability in &self.capabilities {
            validate_dotted_token(&capability.name, "capability_request.capabilities.name")?;
            if let Some(action) = capability.action.as_deref() {
                validate_dotted_token(action, "capability_request.capabilities.action")?;
            }
            if !identities.insert((capability.name.as_str(), capability.action.as_deref())) {
                return Err(SkillSdkError::new(
                    "capability_request_duplicate",
                    format!("name={} action={:?}", capability.name, capability.action),
                ));
            }
            if capability
                .description
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(SkillSdkError::new(
                    "capability_request_description_invalid",
                    format!("name={}", capability.name),
                ));
            }
            if capability
                .timeout_seconds
                .is_some_and(|value| value == 0 || value > 86_400)
            {
                return Err(SkillSdkError::new(
                    "capability_request_timeout_invalid",
                    format!(
                        "name={} timeout={:?}",
                        capability.name, capability.timeout_seconds
                    ),
                ));
            }
            validate_argument_requirements(&capability.required, "required")?;
            validate_argument_requirements(&capability.optional, "optional")?;
            for field in capability.input_roles.keys() {
                validate_argument_field(field, "input_roles")?;
            }
        }
        let mut credential_refs = BTreeSet::new();
        for credential_ref in &self.permissions.credential_refs {
            validate_argument_field(
                credential_ref,
                "capability_request.permissions.credential_refs",
            )?;
            if !credential_refs.insert(credential_ref) {
                return Err(SkillSdkError::new(
                    "capability_request_credential_duplicate",
                    format!("credential_ref={credential_ref}"),
                ));
            }
        }
        validate_tokens(
            &self.artifact_contract.output_fields,
            "capability_request.artifact_contract.output_fields",
        )?;
        validate_tokens(
            &self.evidence_contract.selectors,
            "capability_request.evidence_contract.selectors",
        )?;
        for entry in &self.config_entry_points {
            validate_config_entry_point(entry)?;
        }
        Ok(())
    }
}

fn validate_json_schema(value: &serde_json::Value, field: &str) -> SkillSdkResult<()> {
    if !value.is_object() {
        return Err(SkillSdkError::new(
            "capability_request_schema_invalid",
            format!("field={field}"),
        ));
    }
    Ok(())
}

fn validate_argument_requirements(values: &[String], field: &str) -> SkillSdkResult<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.split('|').any(|alternative| {
            alternative.is_empty()
                || alternative
                    .split('+')
                    .any(|part| validate_argument_field(part, field).is_err())
        }) {
            return Err(SkillSdkError::new(
                "capability_request_argument_invalid",
                format!("field={field} value={value:?}"),
            ));
        }
        if !seen.insert(value) {
            return Err(SkillSdkError::new(
                "capability_request_argument_duplicate",
                format!("field={field} value={value}"),
            ));
        }
    }
    Ok(())
}

fn validate_argument_field(value: &str, field: &str) -> SkillSdkResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(SkillSdkError::new(
            "capability_request_argument_invalid",
            format!("field={field} value={value:?}"),
        ));
    }
    Ok(())
}

fn validate_tokens(values: &[String], field: &str) -> SkillSdkResult<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_dotted_token(value, field)?;
        if !seen.insert(value) {
            return Err(SkillSdkError::new(
                "capability_request_token_duplicate",
                format!("field={field} value={value}"),
            ));
        }
    }
    Ok(())
}

fn validate_dotted_token(value: &str, field: &str) -> SkillSdkResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment.chars().all(|ch| {
                    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
                })
        });
    if !valid {
        return Err(SkillSdkError::new(
            "capability_request_token_invalid",
            format!("field={field} value={value:?}"),
        ));
    }
    Ok(())
}

fn validate_config_entry_point(entry: &ConfigEntryPointRequest) -> SkillSdkResult<()> {
    if entry.reference.trim().is_empty() || entry.reference.len() > 512 {
        return Err(SkillSdkError::new(
            "capability_request_config_entry_invalid",
            format!("kind={:?}", entry.kind),
        ));
    }
    match entry.kind {
        ConfigEntryPointKind::File => validate_relative_path(
            &entry.reference,
            "capability_request.config_entry_points.reference",
            false,
        ),
        ConfigEntryPointKind::Credential => validate_argument_field(
            &entry.reference,
            "capability_request.config_entry_points.reference",
        ),
        ConfigEntryPointKind::Environment => {
            let valid = entry.reference.chars().enumerate().all(|(index, ch)| {
                ch.is_ascii_uppercase() || ch == '_' || (index > 0 && ch.is_ascii_digit())
            });
            if valid {
                Ok(())
            } else {
                Err(SkillSdkError::new(
                    "capability_request_config_entry_invalid",
                    format!("kind=environment reference={:?}", entry.reference),
                ))
            }
        }
        _ if entry.reference.chars().any(char::is_control) => Err(SkillSdkError::new(
            "capability_request_config_entry_invalid",
            format!("kind={:?}", entry.kind),
        )),
        _ => Ok(()),
    }
}
