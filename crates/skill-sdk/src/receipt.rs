use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::manifest::{
    validate_relative_path, validate_safe_name, validate_sha256, BuildAdapter, LauncherKind,
    PackageManifest, SandboxProfile,
};
use crate::platform::HostPlatform;
use crate::{SkillSdkError, SkillSdkResult};

pub const LEGACY_INSTALL_RECEIPT_SCHEMA_VERSION: u32 = 1;
pub const INSTALL_RECEIPT_SCHEMA_VERSION: u32 = 2;
pub const CURRENT_INSTALL_POINTER_SCHEMA_VERSION: u32 = 1;
const BACKGROUND_VERSION_LEASE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackgroundVersionLeaseRecord {
    schema_version: u32,
    job_ref_digest: String,
    expires_at_unix: u64,
}

fn background_version_lease_active(
    record: &BackgroundVersionLeaseRecord,
    now_unix: u64,
) -> Option<bool> {
    (record.schema_version == BACKGROUND_VERSION_LEASE_SCHEMA_VERSION)
        .then_some(record.expires_at_unix > now_unix)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReceipt {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolSmokeReceipt {
    pub protocol: String,
    pub passed: bool,
    pub request_id: String,
    pub checked_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchProgramScope {
    Package,
    TrustedRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptLaunch {
    pub launcher: LauncherKind,
    pub program: String,
    pub program_scope: LaunchProgramScope,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_directory: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Names that the runtime may copy from its already capability-scoped
    /// environment. Values never enter the immutable receipt.
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_runtime_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_runtime_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallReceipt {
    pub schema_version: u32,
    pub skill_name: String,
    pub version: String,
    pub manifest_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_contract_digest: Option<String>,
    pub source_digest: String,
    #[serde(default)]
    pub lockfile_digests: BTreeMap<String, String>,
    pub adapter: BuildAdapter,
    pub adapter_version: String,
    pub platform: HostPlatform,
    pub artifacts: Vec<ArtifactReceipt>,
    pub launch: ReceiptLaunch,
    pub sandbox_profile: SandboxProfile,
    #[serde(default)]
    pub runtime_network: bool,
    pub protocol_smoke: ProtocolSmokeReceipt,
    pub installed_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentInstallPointer {
    pub schema_version: u32,
    pub version: String,
    pub install_dir: String,
    pub receipt_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedInstall {
    pub install_dir: PathBuf,
    pub manifest: PackageManifest,
    pub receipt: InstallReceipt,
}

impl InstallReceipt {
    pub fn validate(&self) -> SkillSdkResult<()> {
        if !matches!(
            self.schema_version,
            LEGACY_INSTALL_RECEIPT_SCHEMA_VERSION | INSTALL_RECEIPT_SCHEMA_VERSION
        ) {
            return Err(SkillSdkError::new(
                "receipt_schema_unsupported",
                format!("schema_version={}", self.schema_version),
            ));
        }
        validate_safe_name(&self.skill_name, "receipt.skill_name")?;
        validate_sha256(&self.manifest_digest, "receipt.manifest_digest")?;
        match (
            self.schema_version,
            self.semantic_contract_digest.as_deref(),
        ) {
            (INSTALL_RECEIPT_SCHEMA_VERSION, Some(digest)) => {
                validate_sha256(digest, "receipt.semantic_contract_digest")?;
            }
            (INSTALL_RECEIPT_SCHEMA_VERSION, None) => {
                return Err(SkillSdkError::new(
                    "receipt_semantic_contract_missing",
                    format!("skill={}", self.skill_name),
                ));
            }
            (LEGACY_INSTALL_RECEIPT_SCHEMA_VERSION, Some(_)) => {
                return Err(SkillSdkError::new(
                    "receipt_semantic_contract_unexpected",
                    format!("skill={}", self.skill_name),
                ));
            }
            _ => {}
        }
        validate_sha256(&self.source_digest, "receipt.source_digest")?;
        if self.version.trim().is_empty()
            || self.adapter_version.trim().is_empty()
            || !self.protocol_smoke.passed
            || self.protocol_smoke.request_id.trim().is_empty()
        {
            return Err(SkillSdkError::new(
                "receipt_required_field_missing",
                format!("skill={}", self.skill_name),
            ));
        }
        for (path, digest) in &self.lockfile_digests {
            validate_relative_path(path, "receipt.lockfile", false)?;
            validate_sha256(digest, "receipt.lockfile_digest")?;
        }
        if self.artifacts.is_empty() && self.adapter != BuildAdapter::HttpJson {
            return Err(SkillSdkError::new(
                "receipt_artifact_missing",
                format!("skill={}", self.skill_name),
            ));
        }
        for artifact in &self.artifacts {
            validate_relative_path(&artifact.path, "receipt.artifact.path", false)?;
            validate_sha256(&artifact.sha256, "receipt.artifact.sha256")?;
        }
        match self.launch.program_scope {
            LaunchProgramScope::Package => {
                validate_relative_path(&self.launch.program, "receipt.launch.program", false)?;
            }
            LaunchProgramScope::TrustedRuntime => {
                let path = Path::new(&self.launch.program);
                if !path.is_absolute() {
                    return Err(SkillSdkError::new(
                        "receipt_runtime_path_invalid",
                        format!("program={}", self.launch.program),
                    ));
                }
                validate_sha256(
                    self.launch
                        .trusted_runtime_sha256
                        .as_deref()
                        .unwrap_or_default(),
                    "receipt.launch.trusted_runtime_sha256",
                )?;
            }
        }
        match self.launch.launcher {
            LauncherKind::HttpJson => {
                if !self
                    .launch
                    .remote_endpoint
                    .as_deref()
                    .is_some_and(|endpoint| endpoint.starts_with("https://"))
                {
                    return Err(SkillSdkError::new(
                        "receipt_remote_endpoint_invalid",
                        "http_json launch requires an HTTPS endpoint",
                    ));
                }
            }
            _ if self.launch.remote_endpoint.is_some() => {
                return Err(SkillSdkError::new(
                    "receipt_remote_endpoint_unexpected",
                    "only http_json launch may declare remote_endpoint",
                ));
            }
            _ => {}
        }
        validate_relative_path(
            &self.launch.working_directory,
            "receipt.launch.working_directory",
            true,
        )?;
        for key in self.launch.environment.keys() {
            if is_sensitive_runtime_environment_name(key) {
                return Err(SkillSdkError::new(
                    "receipt_secret_forbidden",
                    format!("environment_key={key}"),
                ));
            }
        }
        let mut environment_names = std::collections::BTreeSet::new();
        for key in &self.launch.environment_allowlist {
            if !valid_environment_name(key) || !environment_names.insert(key) {
                return Err(SkillSdkError::new(
                    "receipt_environment_allowlist_invalid",
                    format!("environment_key={key}"),
                ));
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> SkillSdkResult<String> {
        self.validate()?;
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(self)?)))
    }

    pub fn artifact_set_digest(&self) -> SkillSdkResult<String> {
        self.validate()?;
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(
            &self.artifacts,
        )?)))
    }

    pub fn verifies_manifest(&self, manifest: &PackageManifest) -> SkillSdkResult<()> {
        if self.skill_name != manifest.package.name || self.version != manifest.package.version {
            return Err(SkillSdkError::new(
                "receipt_manifest_identity_mismatch",
                format!(
                    "receipt={}:{} manifest={}:{}",
                    self.skill_name, self.version, manifest.package.name, manifest.package.version
                ),
            ));
        }
        let digest = manifest.digest()?;
        if digest != self.manifest_digest {
            return Err(SkillSdkError::new(
                "receipt_manifest_digest_mismatch",
                format!("expected={} actual={digest}", self.manifest_digest),
            ));
        }
        if self.schema_version == INSTALL_RECEIPT_SCHEMA_VERSION {
            if manifest.schema_version != crate::manifest::SKILL_MANIFEST_SCHEMA_VERSION {
                return Err(SkillSdkError::new(
                    "receipt_manifest_schema_mismatch",
                    format!(
                        "receipt_schema={} manifest_schema={}",
                        self.schema_version, manifest.schema_version
                    ),
                ));
            }
            let semantic_digest = manifest.capability_request_digest()?;
            if self.semantic_contract_digest.as_deref() != Some(semantic_digest.as_str()) {
                return Err(SkillSdkError::new(
                    "receipt_semantic_contract_mismatch",
                    format!("skill={}", self.skill_name),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct InstallReceiptStore {
    root: PathBuf,
}

#[derive(Debug)]
pub struct SkillVersionLease {
    file: File,
    store: InstallReceiptStore,
    skill_name: String,
    install_dir: String,
}

impl SkillVersionLease {
    pub fn install_dir(&self) -> &str {
        &self.install_dir
    }
}

impl Drop for SkillVersionLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        let skill_root = match self.store.skill_root(&self.skill_name) {
            Ok(root) => root,
            Err(_) => return,
        };
        if skill_root.join("gc-requested").is_file() {
            let _ = self
                .store
                .garbage_collect_installed_versions(&self.skill_name);
        } else {
            let _ = self
                .store
                .garbage_collect_superseded_versions(&self.skill_name);
        }
    }
}

impl InstallReceiptStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn skill_root(&self, skill_name: &str) -> SkillSdkResult<PathBuf> {
        validate_safe_name(skill_name, "skill_name")?;
        Ok(self.root.join(skill_name))
    }

    pub fn create_staging_dir(&self, skill_name: &str) -> SkillSdkResult<PathBuf> {
        let root = self.skill_root(skill_name)?.join("staging");
        fs::create_dir_all(&root)?;
        let path = root.join(Uuid::new_v4().to_string());
        fs::create_dir(&path)?;
        Ok(path)
    }

    pub fn version_dir(
        &self,
        skill_name: &str,
        version: &str,
        manifest_digest: &str,
    ) -> SkillSdkResult<PathBuf> {
        validate_safe_name(skill_name, "skill_name")?;
        validate_sha256(manifest_digest, "manifest_digest")?;
        if version.trim().is_empty()
            || version.contains('/')
            || version.contains('\\')
            || version == "."
            || version == ".."
        {
            return Err(SkillSdkError::new(
                "receipt_version_invalid",
                format!("version={version:?}"),
            ));
        }
        Ok(self
            .skill_root(skill_name)?
            .join("versions")
            .join(format!("{version}-{}", &manifest_digest[..12])))
    }

    pub fn write_receipt(
        &self,
        install_dir: &Path,
        receipt: &InstallReceipt,
    ) -> SkillSdkResult<PathBuf> {
        receipt.validate()?;
        fs::create_dir_all(install_dir)?;
        let destination = install_dir.join("install-receipt.json");
        atomic_write_json(&destination, receipt)?;
        Ok(destination)
    }

    pub fn activate(&self, install_dir: &Path, receipt: &InstallReceipt) -> SkillSdkResult<()> {
        receipt.validate()?;
        let skill_root = self.skill_root(&receipt.skill_name)?;
        let versions_root = skill_root.join("versions");
        fs::create_dir_all(&versions_root)?;
        let canonical_versions = fs::canonicalize(&versions_root)?;
        let canonical_install = fs::canonicalize(install_dir)?;
        if !canonical_install.starts_with(&canonical_versions) {
            return Err(SkillSdkError::new(
                "receipt_install_root_escape",
                format!("path={}", install_dir.display()),
            ));
        }
        let install_dir_name = canonical_install
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                SkillSdkError::new(
                    "receipt_install_dir_invalid",
                    canonical_install.display().to_string(),
                )
            })?;
        let current_path = self.active_install_path(&receipt.skill_name)?;
        let previous_path = skill_root.join("previous.json");
        if current_path.is_file() {
            let current = fs::read(&current_path)?;
            atomic_write(&previous_path, &current)?;
            let pointer: CurrentInstallPointer = serde_json::from_slice(&current)?;
            validate_pointer(&pointer)?;
        }
        let pointer = CurrentInstallPointer {
            schema_version: CURRENT_INSTALL_POINTER_SCHEMA_VERSION,
            version: receipt.version.clone(),
            install_dir: install_dir_name.to_string(),
            receipt_digest: receipt.digest()?,
        };
        atomic_write_json(&current_path, &pointer)?;
        Ok(())
    }

    pub fn current_pointer(&self, skill_name: &str) -> SkillSdkResult<CurrentInstallPointer> {
        let path = self.skill_root(skill_name)?.join("current.json");
        let pointer: CurrentInstallPointer = serde_json::from_slice(&fs::read(&path)?)?;
        validate_pointer(&pointer)?;
        Ok(pointer)
    }

    pub fn verified_current_install(&self, skill_name: &str) -> SkillSdkResult<VerifiedInstall> {
        let pointer = self.current_pointer(skill_name)?;
        self.verify_pointer_install(skill_name, &pointer)
    }

    pub fn rollback(&self, skill_name: &str) -> SkillSdkResult<CurrentInstallPointer> {
        let skill_root = self.skill_root(skill_name)?;
        let current_path = self.active_install_path(skill_name)?;
        let previous_path = skill_root.join("previous.json");
        let previous: CurrentInstallPointer = serde_json::from_slice(&fs::read(&previous_path)?)?;
        validate_pointer(&previous)?;
        self.verify_pointer_install(skill_name, &previous)?;
        let current = fs::read(&current_path)?;
        atomic_write_json(&current_path, &previous)?;
        atomic_write(&previous_path, &current)?;
        Ok(previous)
    }

    /// Remove the mutable current pointer without deleting an immutable
    /// installed version. This is used to roll back a failed first-time
    /// installation after activation but before the surrounding host
    /// transaction completes.
    pub fn deactivate_current(
        &self,
        skill_name: &str,
    ) -> SkillSdkResult<Option<CurrentInstallPointer>> {
        let skill_root = self.skill_root(skill_name)?;
        let current_path = skill_root.join("current.json");
        let raw = match fs::read(&current_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let current: CurrentInstallPointer = serde_json::from_slice(&raw)?;
        validate_pointer(&current)?;
        self.verify_pointer_install(skill_name, &current)?;
        fs::remove_file(current_path)?;
        Ok(Some(current))
    }

    pub fn acquire_version_lease(
        &self,
        skill_name: &str,
        install_root: &Path,
    ) -> SkillSdkResult<SkillVersionLease> {
        let versions_root = self.skill_root(skill_name)?.join("versions");
        let canonical_versions = fs::canonicalize(&versions_root)?;
        let canonical_install = fs::canonicalize(install_root)?;
        if !canonical_install.starts_with(&canonical_versions)
            || canonical_install.parent() != Some(canonical_versions.as_path())
        {
            return Err(SkillSdkError::new(
                "skill_lease_install_root_invalid",
                canonical_install.display().to_string(),
            ));
        }
        let install_dir = canonical_install
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                SkillSdkError::new(
                    "skill_lease_install_dir_invalid",
                    canonical_install.display().to_string(),
                )
            })?
            .to_string();
        let leases_root = self.skill_root(skill_name)?.join("leases");
        fs::create_dir_all(&leases_root)?;
        let path = leases_root.join(format!("{install_dir}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)?;
        FileExt::lock_shared(&file)?;
        if !canonical_install.is_dir() {
            let _ = FileExt::unlock(&file);
            return Err(SkillSdkError::new(
                "skill_lease_install_removed",
                canonical_install.display().to_string(),
            ));
        }
        Ok(SkillVersionLease {
            file,
            store: self.clone(),
            skill_name: skill_name.to_string(),
            install_dir,
        })
    }

    pub fn retain_background_version_lease(
        &self,
        skill_name: &str,
        install_root: &Path,
        job_ref: &str,
        expires_at_unix: u64,
        now_unix: u64,
    ) -> SkillSdkResult<()> {
        if job_ref.trim().is_empty() || expires_at_unix <= now_unix {
            return Err(SkillSdkError::new(
                "skill_background_lease_invalid",
                format!("skill={skill_name} expires_at_unix={expires_at_unix}"),
            ));
        }
        let install_dir = self.background_lease_install_dir(skill_name, install_root)?;
        let job_ref_digest = hex::encode(Sha256::digest(job_ref.trim().as_bytes()));
        let lease_root = self
            .skill_root(skill_name)?
            .join("background-leases")
            .join(install_dir);
        fs::create_dir_all(&lease_root)?;
        atomic_write_json(
            &lease_root.join(format!("{job_ref_digest}.json")),
            &BackgroundVersionLeaseRecord {
                schema_version: BACKGROUND_VERSION_LEASE_SCHEMA_VERSION,
                job_ref_digest,
                expires_at_unix,
            },
        )
    }

    pub fn release_background_version_lease(
        &self,
        skill_name: &str,
        install_root: &Path,
        job_ref: &str,
    ) -> SkillSdkResult<()> {
        if job_ref.trim().is_empty() {
            return Err(SkillSdkError::new(
                "skill_background_lease_invalid",
                format!("skill={skill_name}"),
            ));
        }
        let install_dir = self.background_lease_install_dir(skill_name, install_root)?;
        let job_ref_digest = hex::encode(Sha256::digest(job_ref.trim().as_bytes()));
        let leases_root = self.skill_root(skill_name)?.join("background-leases");
        let install_leases = leases_root.join(install_dir);
        match fs::remove_file(install_leases.join(format!("{job_ref_digest}.json"))) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        remove_empty_directory(&install_leases)?;
        remove_empty_directory(&leases_root)?;
        Ok(())
    }

    fn background_lease_install_dir(
        &self,
        skill_name: &str,
        install_root: &Path,
    ) -> SkillSdkResult<String> {
        let versions_root = self.skill_root(skill_name)?.join("versions");
        let canonical_versions = fs::canonicalize(&versions_root)?;
        let canonical_install = fs::canonicalize(install_root)?;
        if !canonical_install.starts_with(&canonical_versions)
            || canonical_install.parent() != Some(canonical_versions.as_path())
        {
            return Err(SkillSdkError::new(
                "skill_background_lease_install_root_invalid",
                canonical_install.display().to_string(),
            ));
        }
        canonical_install
            .file_name()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                SkillSdkError::new(
                    "skill_background_lease_install_dir_invalid",
                    canonical_install.display().to_string(),
                )
            })
    }

    fn version_has_active_background_lease(
        &self,
        skill_name: &str,
        install_dir: &str,
    ) -> SkillSdkResult<bool> {
        let lease_root = self
            .skill_root(skill_name)?
            .join("background-leases")
            .join(install_dir);
        if !lease_root.is_dir() {
            return Ok(false);
        }
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| SkillSdkError::new("system_time_before_epoch", error.to_string()))?
            .as_secs();
        let mut active = false;
        for entry in fs::read_dir(&lease_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                active = true;
                continue;
            }
            let record = fs::read(entry.path())
                .ok()
                .and_then(|raw| serde_json::from_slice::<BackgroundVersionLeaseRecord>(&raw).ok());
            match record
                .as_ref()
                .and_then(|record| background_version_lease_active(record, now_unix))
            {
                Some(true) => active = true,
                Some(false) => {
                    let _ = fs::remove_file(entry.path());
                }
                None => active = true,
            }
        }
        if !active {
            remove_empty_directory(&lease_root)?;
            remove_empty_directory(&self.skill_root(skill_name)?.join("background-leases"))?;
        }
        Ok(active)
    }

    fn active_install_path(&self, skill_name: &str) -> SkillSdkResult<PathBuf> {
        Ok(self.skill_root(skill_name)?.join("current.json"))
    }

    fn verify_pointer_install(
        &self,
        skill_name: &str,
        pointer: &CurrentInstallPointer,
    ) -> SkillSdkResult<VerifiedInstall> {
        let versions_root = self.skill_root(skill_name)?.join("versions");
        let canonical_versions = fs::canonicalize(&versions_root)?;
        let install_root = fs::canonicalize(versions_root.join(&pointer.install_dir))?;
        if !install_root.starts_with(&canonical_versions) {
            return Err(SkillSdkError::new(
                "receipt_pointer_path_invalid",
                format!("install_dir={}", pointer.install_dir),
            ));
        }
        let receipt: InstallReceipt =
            serde_json::from_slice(&fs::read(install_root.join("install-receipt.json"))?)?;
        receipt.validate()?;
        if receipt.skill_name != skill_name || receipt.digest()? != pointer.receipt_digest {
            return Err(SkillSdkError::new(
                "rollback_receipt_mismatch",
                format!("skill={skill_name} install_dir={}", pointer.install_dir),
            ));
        }
        let manifest = PackageManifest::load(&install_root.join("skill.toml"))?;
        receipt.verifies_manifest(&manifest)?;
        for artifact in &receipt.artifacts {
            let path = fs::canonicalize(install_root.join(&artifact.path))?;
            let metadata = fs::metadata(&path)?;
            if !path.starts_with(&install_root)
                || !metadata.is_file()
                || metadata.len() != artifact.size_bytes
                || digest_file(&path)? != artifact.sha256
            {
                return Err(SkillSdkError::new(
                    "rollback_artifact_mismatch",
                    format!("path={}", artifact.path),
                ));
            }
        }
        Ok(VerifiedInstall {
            install_dir: install_root,
            manifest,
            receipt,
        })
    }

    pub fn remove_installed_versions(&self, skill_name: &str) -> SkillSdkResult<bool> {
        let skill_root = self.skill_root(skill_name)?;
        if !skill_root.exists() {
            return Ok(false);
        }
        fs::write(skill_root.join("gc-requested"), b"pending\n")?;
        self.garbage_collect_installed_versions(skill_name)
    }

    fn garbage_collect_installed_versions(&self, skill_name: &str) -> SkillSdkResult<bool> {
        let skill_root = self.skill_root(skill_name)?;
        if !skill_root.exists() {
            return Ok(false);
        }
        let versions_root = skill_root.join("versions");
        let leases_root = skill_root.join("leases");
        let mut retained = false;
        if versions_root.is_dir() {
            for entry in fs::read_dir(&versions_root)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let install_dir = entry.file_name().to_string_lossy().to_string();
                if self.version_has_active_background_lease(skill_name, &install_dir)? {
                    retained = true;
                    continue;
                }
                fs::create_dir_all(&leases_root)?;
                let lease_path = leases_root.join(format!("{install_dir}.lock"));
                let lease = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&lease_path)?;
                match FileExt::try_lock_exclusive(&lease) {
                    Ok(()) => {
                        fs::remove_dir_all(entry.path())?;
                        let _ = FileExt::unlock(&lease);
                        drop(lease);
                        let _ = fs::remove_file(lease_path);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        retained = true;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if retained {
            return Ok(false);
        }
        fs::remove_dir_all(skill_root)?;
        Ok(true)
    }

    fn garbage_collect_superseded_versions(&self, skill_name: &str) -> SkillSdkResult<()> {
        let skill_root = self.skill_root(skill_name)?;
        let pending_root = skill_root.join("gc-versions");
        if !pending_root.is_dir() {
            return Ok(());
        }
        let current = self.current_pointer(skill_name).ok();
        let versions_root = skill_root.join("versions");
        let leases_root = skill_root.join("leases");
        for entry in fs::read_dir(&pending_root)? {
            let entry = entry?;
            let install_dir = entry.file_name().to_string_lossy().to_string();
            if current
                .as_ref()
                .is_some_and(|pointer| pointer.install_dir == install_dir)
            {
                continue;
            }
            if self.version_has_active_background_lease(skill_name, &install_dir)? {
                continue;
            }
            fs::create_dir_all(&leases_root)?;
            let lease_path = leases_root.join(format!("{install_dir}.lock"));
            let lease = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&lease_path)?;
            match FileExt::try_lock_exclusive(&lease) {
                Ok(()) => {
                    let version_root = versions_root.join(&install_dir);
                    if version_root.is_dir() {
                        fs::remove_dir_all(version_root)?;
                    }
                    let _ = FileExt::unlock(&lease);
                    drop(lease);
                    let _ = fs::remove_file(lease_path);
                    fs::remove_file(entry.path())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
        if fs::read_dir(&pending_root)?.next().is_none() {
            fs::remove_dir(&pending_root)?;
        }
        Ok(())
    }
}

pub fn digest_file(path: &Path) -> SkillSdkResult<String> {
    let mut file = File::open(path).map_err(|error| {
        SkillSdkError::new(
            "artifact_read_failed",
            format!("path={} error={error}", path.display()),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            SkillSdkError::new(
                "artifact_read_failed",
                format!("path={} error={error}", path.display()),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn validate_pointer(pointer: &CurrentInstallPointer) -> SkillSdkResult<()> {
    if pointer.schema_version != CURRENT_INSTALL_POINTER_SCHEMA_VERSION {
        return Err(SkillSdkError::new(
            "receipt_pointer_schema_unsupported",
            format!("schema_version={}", pointer.schema_version),
        ));
    }
    validate_relative_path(&pointer.install_dir, "current.install_dir", false)?;
    if Path::new(&pointer.install_dir).components().count() != 1 {
        return Err(SkillSdkError::new(
            "receipt_pointer_path_invalid",
            format!("install_dir={}", pointer.install_dir),
        ));
    }
    validate_sha256(&pointer.receipt_digest, "current.receipt_digest")
}

pub fn is_sensitive_runtime_environment_name(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "CREDENTIAL",
        "API_KEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "COOKIE",
        "BEARER",
        "AUTH",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

fn valid_environment_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_uppercase())
        && chars.all(|ch| ch == '_' || ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> SkillSdkResult<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_write(path, &bytes)
}

fn remove_empty_directory(path: &Path) -> SkillSdkResult<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> SkillSdkResult<()> {
    let parent = path.parent().ok_or_else(|| {
        SkillSdkError::new("atomic_write_parent_missing", path.display().to_string())
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        SkillSdkError::new(
            "atomic_write_commit_failed",
            format!("path={} error={error}", path.display()),
        )
    })
}

#[cfg(test)]
mod background_version_lease_tests {
    use super::*;

    #[test]
    fn multi_day_lease_uses_explicit_expiry_and_unknown_schema_fails_safe() {
        let start = 10_000;
        let record = BackgroundVersionLeaseRecord {
            schema_version: BACKGROUND_VERSION_LEASE_SCHEMA_VERSION,
            job_ref_digest: "a".repeat(64),
            expires_at_unix: start + 14 * 86_400,
        };
        assert_eq!(
            background_version_lease_active(&record, start + 13 * 86_400),
            Some(true)
        );
        assert_eq!(
            background_version_lease_active(&record, start + 15 * 86_400),
            Some(false)
        );

        let unknown = BackgroundVersionLeaseRecord {
            schema_version: BACKGROUND_VERSION_LEASE_SCHEMA_VERSION + 1,
            ..record
        };
        assert_eq!(background_version_lease_active(&unknown, start), None);
    }
}
