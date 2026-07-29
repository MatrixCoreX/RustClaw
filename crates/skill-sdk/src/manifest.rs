use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capability_request::{
    ArtifactContractRequest, CapabilityActionRequest, CapabilityRequestSet, ConfigEntryPointKind,
    ConfigEntryPointRequest, EvidenceContractRequest, RequestedEffect, RequestedExecutionMode,
    RuntimePermissionRequest, CAPABILITY_REQUEST_SCHEMA_VERSION,
};
use crate::platform::{normalize_arch, normalize_os, HostPlatform};
use crate::{SkillSdkError, SkillSdkResult};

pub const LEGACY_SKILL_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const SKILL_MANIFEST_SCHEMA_VERSION: u32 = 2;
pub const RUSTCLAW_JSONL_PROTOCOL: &str = "rustclaw-jsonl-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildAdapter {
    Cargo,
    Python,
    Node,
    Go,
    Prebuilt,
    GenericProcess,
    HttpJson,
}

impl BuildAdapter {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Python => "python",
            Self::Node => "node",
            Self::Go => "go",
            Self::Prebuilt => "prebuilt",
            Self::GenericProcess => "generic_process",
            Self::HttpJson => "http_json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    Native,
    Python,
    Node,
    Java,
    Dotnet,
    Process,
    HttpJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildNetworkPolicy {
    Deny,
    ApprovalRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

impl Default for BuildNetworkPolicy {
    fn default() -> Self {
        Self::Deny
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    Required,
    ReadOnly,
    WorkspaceWrite,
    Networked,
}

impl Default for SandboxProfile {
    fn default() -> Self {
        Self::Required
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub schema_version: u32,
    pub package: PackageMetadata,
    pub registry: RegistryReference,
    pub build: BuildSpec,
    pub run: RunSpec,
    pub security: SecuritySpec,
    #[serde(default)]
    pub storage: StorageSpec,
    #[serde(default)]
    pub lifecycle: LifecycleSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_request: Option<CapabilityRequestSet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub protocol: String,
    pub supported_os: Vec<String>,
    pub supported_arch: Vec<String>,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryReference {
    pub name: String,
    #[serde(default = "registry_policy_source")]
    pub capability_policy_source: String,
}

fn registry_policy_source() -> String {
    "registry".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSpec {
    pub adapter: BuildAdapter,
    #[serde(default = "current_dir")]
    pub source_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lockfile: Option<String>,
    #[serde(default)]
    pub network: BuildNetworkPolicy,
    #[serde(default)]
    pub lifecycle_scripts: bool,
    #[serde(default)]
    pub artifacts: Vec<PlatformArtifact>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

fn current_dir() -> String {
    ".".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformArtifact {
    pub os: String,
    pub arch: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub executable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub launcher: LauncherKind,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "working_directory_root")]
    pub working_directory: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    #[serde(default = "empty_object")]
    pub smoke_args: serde_json::Value,
}

fn working_directory_root() -> String {
    ".".to_string()
}

fn default_timeout_seconds() -> u64 {
    30
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecuritySpec {
    #[serde(default = "registry_policy_source")]
    pub capability_policy_source: String,
    #[serde(default)]
    pub sandbox: SandboxProfile,
    #[serde(default)]
    pub runtime_network: bool,
    #[serde(default)]
    pub inherit_credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSpec {
    pub kind: String,
    pub schema_version: u32,
    pub migration_owner: String,
}

impl Default for StorageSpec {
    fn default() -> Self {
        Self {
            kind: "none".to_string(),
            schema_version: 1,
            migration_owner: "none".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSpec {
    #[serde(default)]
    pub config_files: Vec<String>,
    #[serde(default)]
    pub preserve_data_on_uninstall: bool,
    #[serde(default = "replace_update_strategy")]
    pub update_strategy: String,
}

fn replace_update_strategy() -> String {
    "atomic_replace".to_string()
}

impl PackageManifest {
    pub fn from_toml_str(raw: &str) -> SkillSdkResult<Self> {
        let manifest: Self = toml::from_str(raw).map_err(|error| {
            SkillSdkError::new("manifest_parse_failed", error.to_string()).phase("manifest")
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> SkillSdkResult<Self> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            SkillSdkError::new(
                "manifest_read_failed",
                format!("path={} error={error}", path.display()),
            )
            .phase("manifest")
        })?;
        Self::from_toml_str(&raw)
    }

    pub fn to_toml_string(&self) -> SkillSdkResult<String> {
        self.validate()?;
        toml::to_string_pretty(self)
            .map_err(|error| SkillSdkError::new("manifest_encode_failed", error.to_string()))
    }

    pub fn digest(&self) -> SkillSdkResult<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    pub fn capability_request_digest(&self) -> SkillSdkResult<String> {
        let request = self.effective_capability_request()?;
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&request)?)))
    }

    pub fn effective_capability_request(&self) -> SkillSdkResult<CapabilityRequestSet> {
        match self.schema_version {
            SKILL_MANIFEST_SCHEMA_VERSION => self.capability_request.clone().ok_or_else(|| {
                SkillSdkError::new(
                    "capability_request_missing",
                    "schema version 2 requires capability_request",
                )
            }),
            LEGACY_SKILL_MANIFEST_SCHEMA_VERSION => Ok(self.legacy_capability_request()),
            _ => Err(SkillSdkError::new(
                "manifest_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            )),
        }
    }

    pub fn requested_runtime_network(&self) -> SkillSdkResult<bool> {
        Ok(self.effective_capability_request()?.permissions.network)
    }

    pub fn into_current(mut self) -> SkillSdkResult<Self> {
        if self.schema_version == LEGACY_SKILL_MANIFEST_SCHEMA_VERSION {
            self.capability_request = Some(self.legacy_capability_request());
            self.schema_version = SKILL_MANIFEST_SCHEMA_VERSION;
            self.security.runtime_network = false;
        }
        self.validate()?;
        Ok(self)
    }

    pub fn validate_for_platform(&self, platform: &HostPlatform) -> SkillSdkResult<()> {
        self.validate()?;
        if !platform.matches(&self.package.supported_os, &self.package.supported_arch) {
            return Err(SkillSdkError::new(
                "platform_unsupported",
                format!(
                    "skill={} os={} arch={}",
                    self.package.name, platform.os, platform.arch
                ),
            )
            .phase("preflight"));
        }
        Ok(())
    }

    pub fn validate(&self) -> SkillSdkResult<()> {
        if !matches!(
            self.schema_version,
            LEGACY_SKILL_MANIFEST_SCHEMA_VERSION | SKILL_MANIFEST_SCHEMA_VERSION
        ) {
            return Err(SkillSdkError::new(
                "manifest_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        match self.schema_version {
            LEGACY_SKILL_MANIFEST_SCHEMA_VERSION if self.capability_request.is_some() => {
                return Err(SkillSdkError::new(
                    "capability_request_unexpected",
                    "schema version 1 is read-only compatibility and cannot declare capability_request",
                ));
            }
            SKILL_MANIFEST_SCHEMA_VERSION => {
                self.capability_request
                    .as_ref()
                    .ok_or_else(|| {
                        SkillSdkError::new(
                            "capability_request_missing",
                            "schema version 2 requires capability_request",
                        )
                    })?
                    .validate()?;
                if self.security.runtime_network {
                    return Err(SkillSdkError::new(
                        "manifest_grant_field_forbidden",
                        "schema version 2 must request network under capability_request.permissions; security.runtime_network is a legacy grant-shaped field",
                    ));
                }
            }
            _ => {}
        }
        validate_safe_name(&self.package.name, "package.name")?;
        validate_safe_name(&self.registry.name, "registry.name")?;
        if self.package.name != self.registry.name {
            return Err(SkillSdkError::new(
                "manifest_registry_name_mismatch",
                format!(
                    "package={} registry={}",
                    self.package.name, self.registry.name
                ),
            ));
        }
        if self.package.version.trim().is_empty()
            || self.package.version.contains(char::is_whitespace)
        {
            return Err(SkillSdkError::new(
                "manifest_version_invalid",
                format!("version={:?}", self.package.version),
            ));
        }
        if self.package.description.trim().is_empty() || self.package.license.trim().is_empty() {
            return Err(SkillSdkError::new(
                "manifest_metadata_missing",
                "description and license are required",
            ));
        }
        if self.package.protocol != RUSTCLAW_JSONL_PROTOCOL {
            return Err(SkillSdkError::new(
                "manifest_protocol_unsupported",
                format!("protocol={}", self.package.protocol),
            ));
        }
        validate_platform_tokens(&self.package.supported_os, normalize_os, "supported_os")?;
        validate_platform_tokens(
            &self.package.supported_arch,
            normalize_arch,
            "supported_arch",
        )?;
        validate_relative_path(&self.build.source_root, "build.source_root", true)?;
        validate_relative_path(&self.run.entrypoint, "run.entrypoint", false)?;
        validate_relative_path(&self.run.working_directory, "run.working_directory", true)?;
        if let Some(lockfile) = self.build.lockfile.as_deref() {
            validate_relative_path(lockfile, "build.lockfile", false)?;
        }
        for config in &self.lifecycle.config_files {
            validate_relative_path(config, "lifecycle.config_files", false)?;
        }
        if self.run.timeout_seconds == 0 || self.run.timeout_seconds > 86_400 {
            return Err(SkillSdkError::new(
                "manifest_timeout_invalid",
                format!("timeout_seconds={}", self.run.timeout_seconds),
            ));
        }
        for variable in &self.run.environment_allowlist {
            if !valid_environment_name(variable) {
                return Err(SkillSdkError::new(
                    "manifest_environment_invalid",
                    format!("variable={variable}"),
                ));
            }
        }
        if !self.run.smoke_args.is_object() {
            return Err(SkillSdkError::new(
                "manifest_smoke_args_invalid",
                "run.smoke_args must be an object",
            ));
        }
        if self.security.inherit_credentials {
            return Err(SkillSdkError::new(
                "manifest_credential_inheritance_forbidden",
                "skills must receive scoped secret references from runtime",
            ));
        }
        if self.build.lifecycle_scripts {
            return Err(SkillSdkError::new(
                "manifest_lifecycle_scripts_forbidden",
                "dependency lifecycle scripts are not supported",
            ));
        }
        if self.registry.capability_policy_source != "registry"
            || self.security.capability_policy_source != "registry"
        {
            return Err(SkillSdkError::new(
                "manifest_policy_source_invalid",
                "capability_policy_source must be registry",
            ));
        }
        self.validate_adapter_fields()?;
        Ok(())
    }

    fn validate_adapter_fields(&self) -> SkillSdkResult<()> {
        if self.build.adapter != BuildAdapter::Cargo
            && (self.build.package.is_some() || self.build.binary.is_some())
        {
            return Err(SkillSdkError::new(
                "manifest_adapter_field_unexpected",
                format!(
                    "adapter={} fields=build.package,build.binary",
                    self.build.adapter.as_token()
                ),
            ));
        }
        if !matches!(
            self.build.adapter,
            BuildAdapter::Cargo | BuildAdapter::Python | BuildAdapter::Node | BuildAdapter::Go
        ) && self.build.lockfile.is_some()
        {
            return Err(SkillSdkError::new(
                "manifest_adapter_field_unexpected",
                format!(
                    "adapter={} field=build.lockfile",
                    self.build.adapter.as_token()
                ),
            ));
        }
        if self.build.adapter != BuildAdapter::Prebuilt && !self.build.artifacts.is_empty() {
            return Err(SkillSdkError::new(
                "manifest_adapter_field_unexpected",
                format!(
                    "adapter={} field=build.artifacts",
                    self.build.adapter.as_token()
                ),
            ));
        }
        let (allowed_options, allowed_launchers): (&[&str], &[LauncherKind]) =
            match self.build.adapter {
                BuildAdapter::Cargo => (&[], &[LauncherKind::Native, LauncherKind::Process]),
                BuildAdapter::Python => (&["python"], &[LauncherKind::Python]),
                BuildAdapter::Node => (&["node"], &[LauncherKind::Node]),
                BuildAdapter::Go => (&["main"], &[LauncherKind::Native, LauncherKind::Process]),
                BuildAdapter::Prebuilt => (&[], &[LauncherKind::Native, LauncherKind::Process]),
                BuildAdapter::GenericProcess => (
                    &[],
                    &[
                        LauncherKind::Native,
                        LauncherKind::Process,
                        LauncherKind::Java,
                        LauncherKind::Dotnet,
                    ],
                ),
                BuildAdapter::HttpJson => (&["endpoint"], &[LauncherKind::HttpJson]),
            };
        if let Some(option) = self
            .build
            .options
            .keys()
            .find(|option| !allowed_options.contains(&option.as_str()))
        {
            return Err(SkillSdkError::new(
                "manifest_adapter_option_unknown",
                format!("adapter={} option={option}", self.build.adapter.as_token()),
            ));
        }
        if !allowed_launchers.contains(&self.run.launcher) {
            return Err(SkillSdkError::new(
                "manifest_launcher_mismatch",
                format!(
                    "adapter={} launcher={:?}",
                    self.build.adapter.as_token(),
                    self.run.launcher
                ),
            ));
        }
        match self.build.adapter {
            BuildAdapter::Cargo => {
                validate_safe_name(
                    required_adapter_field(
                        &self.build.package,
                        self.build.adapter,
                        "build.package",
                    )?,
                    "build.package",
                )?;
                validate_safe_name(
                    required_adapter_field(&self.build.binary, self.build.adapter, "build.binary")?,
                    "build.binary",
                )?;
                let lockfile = required_adapter_field(
                    &self.build.lockfile,
                    self.build.adapter,
                    "build.lockfile",
                )?;
                validate_relative_path(lockfile, "build.lockfile", false)?;
            }
            BuildAdapter::Python | BuildAdapter::Node | BuildAdapter::Go => {
                let lockfile = required_adapter_field(
                    &self.build.lockfile,
                    self.build.adapter,
                    "build.lockfile",
                )?;
                validate_relative_path(lockfile, "build.lockfile", false)?;
                if self.build.adapter == BuildAdapter::Go {
                    let main = self
                        .build
                        .options
                        .get("main")
                        .map(String::as_str)
                        .unwrap_or(".");
                    if main.starts_with('-') {
                        return Err(SkillSdkError::new(
                            "manifest_adapter_option_unsafe",
                            format!("adapter=go option=main value={main:?}"),
                        ));
                    }
                    validate_relative_path(main, "build.options.main", true)?;
                }
            }
            BuildAdapter::Prebuilt => {
                if !Path::new(&self.run.entrypoint).starts_with("runtime") {
                    return Err(SkillSdkError::new(
                        "manifest_prebuilt_entrypoint_invalid",
                        "prebuilt entrypoint must stay under runtime/",
                    ));
                }
                if self.build.artifacts.is_empty() {
                    return Err(SkillSdkError::new(
                        "manifest_prebuilt_artifact_missing",
                        "prebuilt adapter requires at least one platform artifact",
                    ));
                }
                for artifact in &self.build.artifacts {
                    if normalize_os(&artifact.os).is_none()
                        || normalize_arch(&artifact.arch).is_none()
                    {
                        return Err(SkillSdkError::new(
                            "manifest_prebuilt_platform_invalid",
                            format!("os={} arch={}", artifact.os, artifact.arch),
                        ));
                    }
                    validate_sha256(&artifact.sha256, "build.artifacts.sha256")?;
                    match (&artifact.source_path, &artifact.url) {
                        (Some(path), None) => {
                            validate_relative_path(path, "build.artifacts.source_path", false)?
                        }
                        (None, Some(url)) if url.starts_with("https://") => {
                            if artifact.size_bytes.is_none() {
                                return Err(SkillSdkError::new(
                                    "manifest_prebuilt_size_missing",
                                    "remote prebuilt artifacts require size_bytes",
                                ));
                            }
                        }
                        _ => {
                            return Err(SkillSdkError::new(
                                "manifest_prebuilt_source_invalid",
                                "provide exactly one relative source_path or https URL",
                            ))
                        }
                    }
                }
            }
            BuildAdapter::GenericProcess => {}
            BuildAdapter::HttpJson => {
                let endpoint = self.build.options.get("endpoint").ok_or_else(|| {
                    SkillSdkError::new(
                        "manifest_adapter_field_missing",
                        "adapter=http_json field=build.options.endpoint",
                    )
                })?;
                let endpoint = reqwest::Url::parse(endpoint).map_err(|error| {
                    SkillSdkError::new("manifest_http_endpoint_invalid", error.to_string())
                })?;
                if endpoint.scheme() != "https"
                    || endpoint.host_str().is_none()
                    || !endpoint.username().is_empty()
                    || endpoint.password().is_some()
                    || self.build.network != BuildNetworkPolicy::ApprovalRequired
                    || !self.requested_runtime_network()?
                {
                    return Err(SkillSdkError::new(
                        "manifest_http_endpoint_invalid",
                        "http_json requires credential-free HTTPS, build.network=approval_required, and runtime_network=true",
                    ));
                }
            }
        }
        Ok(())
    }

    fn legacy_capability_request(&self) -> CapabilityRequestSet {
        let filesystem_write = matches!(
            self.security.sandbox,
            SandboxProfile::WorkspaceWrite | SandboxProfile::Networked
        );
        CapabilityRequestSet {
            schema_version: CAPABILITY_REQUEST_SCHEMA_VERSION,
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            permissions: RuntimePermissionRequest {
                network: self.security.runtime_network
                    || matches!(self.security.sandbox, SandboxProfile::Networked),
                filesystem_write,
                ..RuntimePermissionRequest::default()
            },
            artifact_contract: ArtifactContractRequest {
                kinds: Vec::new(),
                output_fields: Vec::new(),
            },
            evidence_contract: EvidenceContractRequest {
                required: false,
                selectors: Vec::new(),
            },
            config_entry_points: self
                .lifecycle
                .config_files
                .iter()
                .map(|reference| ConfigEntryPointRequest {
                    kind: ConfigEntryPointKind::File,
                    reference: reference.clone(),
                    required: false,
                })
                .collect(),
            capabilities: vec![CapabilityActionRequest {
                name: format!("{}.run", self.package.name),
                action: None,
                description: Some(
                    "Conservative capability request migrated from a schema v1 manifest"
                        .to_string(),
                ),
                effect: RequestedEffect::External,
                execution_mode: RequestedExecutionMode::SyncShort,
                required: Vec::new(),
                optional: Vec::new(),
                input_roles: BTreeMap::new(),
                timeout_seconds: Some(self.run.timeout_seconds),
            }],
        }
    }
}

fn required_adapter_field<'a>(
    value: &'a Option<String>,
    adapter: BuildAdapter,
    field: &str,
) -> SkillSdkResult<&'a str> {
    value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            SkillSdkError::new(
                "manifest_adapter_field_missing",
                format!("adapter={} field={field}", adapter.as_token()),
            )
        })
}

pub fn validate_safe_name(value: &str, field: &str) -> SkillSdkResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        });
    if !valid {
        return Err(SkillSdkError::new(
            "manifest_name_invalid",
            format!("field={field} value={value:?}"),
        ));
    }
    Ok(())
}

pub fn validate_relative_path(value: &str, field: &str, allow_dot: bool) -> SkillSdkResult<()> {
    let path = Path::new(value);
    let invalid = value.is_empty()
        || value.contains('\0')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || (!allow_dot && path == Path::new("."));
    if invalid {
        return Err(SkillSdkError::new(
            "manifest_path_unsafe",
            format!("field={field} value={value:?}"),
        ));
    }
    Ok(())
}

pub fn validate_sha256(value: &str, field: &str) -> SkillSdkResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SkillSdkError::new(
            "manifest_digest_invalid",
            format!("field={field}"),
        ));
    }
    Ok(())
}

fn validate_platform_tokens(
    values: &[String],
    normalize: fn(&str) -> Option<&'static str>,
    field: &str,
) -> SkillSdkResult<()> {
    if values.is_empty()
        || values
            .iter()
            .any(|value| value != "any" && normalize(value).is_none())
    {
        return Err(SkillSdkError::new(
            "manifest_platform_invalid",
            format!("field={field} values={values:?}"),
        ));
    }
    Ok(())
}

fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().enumerate().all(|(index, character)| {
            (index > 0 && character.is_ascii_digit())
                || character.is_ascii_uppercase()
                || character == '_'
        })
}
