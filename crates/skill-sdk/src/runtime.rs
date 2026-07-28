use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{BuildAdapter, LauncherKind, PackageManifest, SandboxProfile};
use crate::receipt::{digest_file, InstallReceipt, InstallReceiptStore, LaunchProgramScope};
use crate::{SkillSdkError, SkillSdkResult};

pub const SKILL_LAUNCH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillLaunchSpec {
    pub schema_version: u32,
    pub skill_name: String,
    pub version: String,
    pub adapter: BuildAdapter,
    pub launcher: LauncherKind,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub environment_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_endpoint: Option<String>,
    pub timeout_seconds: u64,
    pub sandbox_profile: SandboxProfile,
    pub runtime_network: bool,
    pub install_root: PathBuf,
    pub manifest_digest: String,
    pub receipt_digest: String,
}

impl SkillLaunchSpec {
    pub fn validate(&self) -> SkillSdkResult<()> {
        if self.schema_version != SKILL_LAUNCH_SCHEMA_VERSION {
            return Err(SkillSdkError::new(
                "launch_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        if !self.program.is_absolute()
            || !self.working_directory.is_absolute()
            || !self.install_root.is_absolute()
            || !self.program.is_file()
            || !self.working_directory.is_dir()
        {
            return Err(SkillSdkError::new(
                "launch_path_invalid",
                format!(
                    "program={} cwd={} root={}",
                    self.program.display(),
                    self.working_directory.display(),
                    self.install_root.display()
                ),
            ));
        }
        let canonical_root = fs::canonicalize(&self.install_root)?;
        let canonical_cwd = fs::canonicalize(&self.working_directory)?;
        if !canonical_cwd.starts_with(&canonical_root) {
            return Err(SkillSdkError::new(
                "launch_working_directory_escape",
                canonical_cwd.display().to_string(),
            ));
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 86_400 {
            return Err(SkillSdkError::new(
                "launch_timeout_invalid",
                format!("timeout_seconds={}", self.timeout_seconds),
            ));
        }
        if self.args.iter().any(|argument| argument.contains('\0')) {
            return Err(SkillSdkError::new(
                "launch_argument_invalid",
                "argument contains NUL",
            ));
        }
        match self.launcher {
            LauncherKind::HttpJson => {
                if !self
                    .remote_endpoint
                    .as_deref()
                    .is_some_and(|endpoint| endpoint.starts_with("https://"))
                {
                    return Err(SkillSdkError::new(
                        "launch_remote_endpoint_invalid",
                        "http_json launch requires an HTTPS endpoint",
                    ));
                }
            }
            _ if self.remote_endpoint.is_some() => {
                return Err(SkillSdkError::new(
                    "launch_remote_endpoint_unexpected",
                    "only http_json launch may declare remote_endpoint",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SkillRuntimeResolver {
    store: InstallReceiptStore,
}

impl SkillRuntimeResolver {
    pub fn new(package_root: impl Into<PathBuf>) -> Self {
        Self {
            store: InstallReceiptStore::new(package_root),
        }
    }

    pub fn resolve(&self, skill_name: &str) -> SkillSdkResult<SkillLaunchSpec> {
        let pointer = self.store.current_pointer(skill_name).map_err(|error| {
            SkillSdkError::new("launch_receipt_missing", error.to_string()).phase("resolve")
        })?;
        let skill_root = self.store.skill_root(skill_name)?;
        let versions_root = skill_root.join("versions");
        let install_root = versions_root.join(&pointer.install_dir);
        let canonical_versions = fs::canonicalize(&versions_root)?;
        let canonical_install = fs::canonicalize(&install_root)?;
        if !canonical_install.starts_with(&canonical_versions) {
            return Err(SkillSdkError::new(
                "launch_install_root_escape",
                canonical_install.display().to_string(),
            ));
        }
        let manifest = PackageManifest::load(&canonical_install.join("skill.toml"))?;
        let receipt_path = canonical_install.join("install-receipt.json");
        let receipt: InstallReceipt = serde_json::from_slice(&fs::read(&receipt_path)?)?;
        receipt.validate()?;
        receipt.verifies_manifest(&manifest)?;
        let receipt_digest = receipt.digest()?;
        if receipt_digest != pointer.receipt_digest {
            return Err(SkillSdkError::new(
                "launch_receipt_digest_mismatch",
                format!(
                    "expected={} actual={receipt_digest}",
                    pointer.receipt_digest
                ),
            ));
        }
        verify_artifacts(&canonical_install, &receipt)?;
        let program = match receipt.launch.program_scope {
            LaunchProgramScope::Package => {
                confined_existing_path(&canonical_install, &receipt.launch.program, true)?
            }
            LaunchProgramScope::TrustedRuntime => {
                let program = fs::canonicalize(&receipt.launch.program)?;
                let actual = digest_file(&program)?;
                let expected = receipt
                    .launch
                    .trusted_runtime_sha256
                    .as_deref()
                    .unwrap_or_default();
                if actual != expected {
                    return Err(SkillSdkError::new(
                        "launch_runtime_digest_mismatch",
                        format!("program={}", program.display()),
                    ));
                }
                program
            }
        };
        let working_directory =
            confined_existing_path(&canonical_install, &receipt.launch.working_directory, false)?;
        let launch = SkillLaunchSpec {
            schema_version: SKILL_LAUNCH_SCHEMA_VERSION,
            skill_name: receipt.skill_name,
            version: receipt.version,
            adapter: receipt.adapter,
            launcher: receipt.launch.launcher,
            program,
            args: receipt.launch.args,
            working_directory,
            environment: receipt.launch.environment,
            environment_allowlist: receipt.launch.environment_allowlist,
            remote_endpoint: receipt.launch.remote_endpoint,
            timeout_seconds: manifest.run.timeout_seconds,
            sandbox_profile: receipt.sandbox_profile,
            runtime_network: receipt.runtime_network,
            install_root: canonical_install,
            manifest_digest: receipt.manifest_digest,
            receipt_digest,
        };
        launch.validate()?;
        Ok(launch)
    }
}

fn verify_artifacts(install_root: &Path, receipt: &InstallReceipt) -> SkillSdkResult<()> {
    for artifact in &receipt.artifacts {
        let path = confined_existing_path(install_root, &artifact.path, true)?;
        let metadata = fs::metadata(&path)?;
        if metadata.len() != artifact.size_bytes || digest_file(&path)? != artifact.sha256 {
            return Err(SkillSdkError::new(
                "launch_artifact_digest_mismatch",
                format!("path={}", path.display()),
            ));
        }
    }
    Ok(())
}

fn confined_existing_path(root: &Path, relative: &str, file: bool) -> SkillSdkResult<PathBuf> {
    let candidate = fs::canonicalize(root.join(relative)).map_err(|error| {
        SkillSdkError::new(
            "launch_path_missing",
            format!("path={relative} error={error}"),
        )
    })?;
    if !candidate.starts_with(root)
        || (file && !candidate.is_file())
        || (!file && !candidate.is_dir())
    {
        return Err(SkillSdkError::new(
            "launch_path_escape",
            format!("path={}", candidate.display()),
        ));
    }
    Ok(candidate)
}
