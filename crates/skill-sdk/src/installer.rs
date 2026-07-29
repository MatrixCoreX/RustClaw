use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::adapter::{prepare_package, source_digest, AdapterContext, PreparedPackage};
use crate::manifest::{BuildAdapter, BuildNetworkPolicy, PackageManifest, RUSTCLAW_JSONL_PROTOCOL};
use crate::process::run_command_controlled;
use crate::protocol::{validate_response_line, ProtocolRequest};
use crate::receipt::{
    digest_file, ArtifactReceipt, InstallReceipt, InstallReceiptStore, LaunchProgramScope,
    ProtocolSmokeReceipt, ReceiptLaunch, INSTALL_RECEIPT_SCHEMA_VERSION,
};
use crate::sandbox::{prepare_sandboxed_command, SandboxNetwork};
use crate::{HostPlatform, SkillSdkError, SkillSdkResult};

#[derive(Clone)]
pub struct InstallRequest {
    pub manifest_path: PathBuf,
    pub workspace_root: PathBuf,
    pub package_root: PathBuf,
    pub target: Option<String>,
    pub allow_network: bool,
    pub control: Option<InstallControl>,
}

#[derive(Clone)]
pub struct AdoptBuiltRequest {
    pub manifest_path: PathBuf,
    pub workspace_root: PathBuf,
    pub package_root: PathBuf,
    pub binary_path: PathBuf,
    pub target: Option<String>,
    pub control: Option<InstallControl>,
}

#[derive(Clone)]
pub struct PrecompiledInstallRequest {
    pub manifest_path: PathBuf,
    pub workspace_root: PathBuf,
    pub package_root: PathBuf,
    pub precompiled_root: PathBuf,
    pub target: Option<String>,
    pub control: Option<InstallControl>,
}

#[derive(Clone)]
pub struct InstallControl {
    cancelled: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}

impl std::fmt::Debug for InstallControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstallControl")
            .field("cancelled", &self.is_cancelled())
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl InstallControl {
    pub fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            progress: None,
        }
    }

    pub fn with_progress(
        cancelled: Arc<AtomicBool>,
        progress: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self {
            cancelled,
            progress: Some(progress),
        }
    }

    pub fn request_cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn cancelled_flag(&self) -> &AtomicBool {
        &self.cancelled
    }

    pub fn phase(&self, phase: &str) -> SkillSdkResult<()> {
        if self.is_cancelled() {
            return Err(
                SkillSdkError::new("process_cancelled", "cancellation requested").phase(phase),
            );
        }
        if let Some(progress) = &self.progress {
            progress(phase);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallOutcome {
    pub skill_name: String,
    pub version: String,
    pub install_root: PathBuf,
    pub receipt_digest: String,
    pub adapter: BuildAdapter,
    pub origin: InstallOrigin,
    pub reused: bool,
    pub phases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOrigin {
    SourceBuild,
    BuiltArtifact,
    PlatformPrecompiled,
}

#[derive(Debug, Clone, Default)]
pub struct SkillInstaller;

impl SkillInstaller {
    pub fn install(&self, request: &InstallRequest) -> SkillSdkResult<InstallOutcome> {
        let _install_guard = install_process_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        emit_phase(request.control.as_ref(), "preflight")?;
        let workspace_root = fs::canonicalize(&request.workspace_root).map_err(|error| {
            SkillSdkError::new(
                "workspace_root_unavailable",
                format!("path={} error={error}", request.workspace_root.display()),
            )
            .phase("preflight")
        })?;
        let manifest_path = fs::canonicalize(&request.manifest_path).map_err(|error| {
            SkillSdkError::new(
                "manifest_read_failed",
                format!("path={} error={error}", request.manifest_path.display()),
            )
            .phase("manifest")
        })?;
        if !manifest_path.starts_with(&workspace_root) {
            return Err(SkillSdkError::new(
                "manifest_path_escape",
                manifest_path.display().to_string(),
            )
            .phase("preflight"));
        }
        let manifest_dir = manifest_path.parent().ok_or_else(|| {
            SkillSdkError::new(
                "manifest_parent_missing",
                manifest_path.display().to_string(),
            )
        })?;
        let manifest = PackageManifest::load(&manifest_path)?.into_current()?;
        crate::secret_scan::scan_package_source(manifest_dir)?;
        let platform = match request.target.as_deref() {
            Some(target) => HostPlatform::from_target(target)?,
            None => HostPlatform::current(),
        };
        manifest.validate_for_platform(&platform)?;
        let allow_network = matches!(manifest.build.network, BuildNetworkPolicy::ApprovalRequired)
            && request.allow_network;
        fs::create_dir_all(&request.package_root)?;
        let store = InstallReceiptStore::new(&request.package_root);
        let staging = store.create_staging_dir(&manifest.package.name)?;
        let mut guard = StagingGuard::new(staging.clone());
        let cache_root = store
            .root()
            .join("cache")
            .join(manifest.build.adapter.as_token());
        fs::create_dir_all(&cache_root)?;
        let source_digest = source_digest(manifest_dir)?;
        let manifest_digest = manifest.digest()?;
        let context = AdapterContext {
            manifest: &manifest,
            workspace_root: &workspace_root,
            manifest_dir,
            staging_root: &staging,
            cache_root: &cache_root,
            platform: &platform,
            target: request.target.as_deref(),
            allow_network,
            control: request.control.as_ref(),
        };
        let prepared = prepare_package(&context)?;
        fs::write(staging.join("skill.toml"), manifest.to_toml_string()?)?;
        emit_phase(request.control.as_ref(), "protocol_smoke")?;
        let smoke = protocol_smoke(
            &manifest,
            &prepared,
            &staging,
            &workspace_root,
            &platform,
            allow_network,
            request.control.as_ref(),
        )?;
        let receipt = InstallReceipt {
            schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
            skill_name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            manifest_digest: manifest_digest.clone(),
            semantic_contract_digest: Some(manifest.capability_request_digest()?),
            source_digest: source_digest.clone(),
            lockfile_digests: lockfile_digests(&manifest, &workspace_root)?,
            adapter: manifest.build.adapter,
            adapter_version: prepared.adapter_version,
            platform,
            artifacts: prepared.artifacts,
            launch: prepared.launch,
            sandbox_profile: manifest.security.sandbox,
            runtime_network: manifest.requested_runtime_network()?,
            protocol_smoke: smoke,
            installed_at_unix: now_unix()?,
        };
        let mut phases = prepared.phases;
        phases.push("protocol_smoke".to_string());
        phases.push("activate".to_string());
        finish_install(
            &store,
            &staging,
            &mut guard,
            receipt,
            &manifest_digest,
            &source_digest,
            phases,
            InstallOrigin::SourceBuild,
            request.control.as_ref(),
        )
    }

    /// Adopt one already-built Cargo binary into the same immutable receipt
    /// model. This is used by ordinary workspace/release builds so core skills
    /// are not compiled a second time by the installer.
    pub fn adopt_built(&self, request: &AdoptBuiltRequest) -> SkillSdkResult<InstallOutcome> {
        let _install_guard = install_process_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        emit_phase(request.control.as_ref(), "preflight")?;
        let workspace_root = fs::canonicalize(&request.workspace_root).map_err(|error| {
            SkillSdkError::new(
                "workspace_root_unavailable",
                format!("path={} error={error}", request.workspace_root.display()),
            )
            .phase("preflight")
        })?;
        let manifest_path = fs::canonicalize(&request.manifest_path)?;
        let binary_path = fs::canonicalize(&request.binary_path).map_err(|error| {
            SkillSdkError::new(
                "built_artifact_missing",
                format!("path={} error={error}", request.binary_path.display()),
            )
            .phase("preflight")
        })?;
        if !manifest_path.starts_with(&workspace_root) || !binary_path.starts_with(&workspace_root)
        {
            return Err(SkillSdkError::new(
                "adopt_path_escape",
                "manifest and built artifact must stay inside the workspace",
            )
            .phase("preflight"));
        }
        let manifest_dir = manifest_path.parent().ok_or_else(|| {
            SkillSdkError::new(
                "manifest_parent_missing",
                manifest_path.display().to_string(),
            )
        })?;
        let manifest = PackageManifest::load(&manifest_path)?.into_current()?;
        if manifest.build.adapter != BuildAdapter::Cargo {
            return Err(SkillSdkError::new(
                "adopt_adapter_unsupported",
                format!("adapter={}", manifest.build.adapter.as_token()),
            )
            .phase("preflight"));
        }
        let expected_binary =
            manifest.build.binary.as_deref().ok_or_else(|| {
                SkillSdkError::new("manifest_adapter_field_missing", "build.binary")
            })?;
        if binary_path.file_name().and_then(|value| value.to_str()) != Some(expected_binary) {
            return Err(SkillSdkError::new(
                "built_artifact_identity_mismatch",
                format!("expected={expected_binary} path={}", binary_path.display()),
            )
            .phase("preflight"));
        }
        if !binary_path.is_file() {
            return Err(SkillSdkError::new(
                "built_artifact_invalid",
                binary_path.display().to_string(),
            )
            .phase("preflight"));
        }
        crate::secret_scan::scan_package_source(manifest_dir)?;
        let platform = match request.target.as_deref() {
            Some(target) => HostPlatform::from_target(target)?,
            None => HostPlatform::current(),
        };
        manifest.validate_for_platform(&platform)?;
        fs::create_dir_all(&request.package_root)?;
        let store = InstallReceiptStore::new(&request.package_root);
        let staging = store.create_staging_dir(&manifest.package.name)?;
        let mut guard = StagingGuard::new(staging.clone());
        let destination = staging.join(&manifest.run.entrypoint);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&binary_path, &destination)?;
        set_executable(&destination)?;
        emit_phase(request.control.as_ref(), "artifact")?;
        let prepared = PreparedPackage {
            adapter_version: "cargo-adopted-v1".to_string(),
            artifacts: vec![ArtifactReceipt {
                path: manifest.run.entrypoint.clone(),
                sha256: digest_file(&destination)?,
                size_bytes: fs::metadata(&destination)?.len(),
                executable: true,
            }],
            launch: ReceiptLaunch {
                launcher: manifest.run.launcher,
                program: manifest.run.entrypoint.clone(),
                program_scope: LaunchProgramScope::Package,
                args: manifest.run.args.clone(),
                working_directory: manifest.run.working_directory.clone(),
                environment: BTreeMap::new(),
                environment_allowlist: manifest.run.environment_allowlist.clone(),
                trusted_runtime_sha256: None,
                trusted_runtime_version: None,
                remote_endpoint: None,
            },
            phases: vec!["artifact".to_string()],
        };
        fs::write(staging.join("skill.toml"), manifest.to_toml_string()?)?;
        emit_phase(request.control.as_ref(), "protocol_smoke")?;
        let smoke = protocol_smoke(
            &manifest,
            &prepared,
            &staging,
            &workspace_root,
            &platform,
            false,
            request.control.as_ref(),
        )?;
        let manifest_digest = manifest.digest()?;
        let source_digest = source_digest(manifest_dir)?;
        let receipt = InstallReceipt {
            schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
            skill_name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            manifest_digest: manifest_digest.clone(),
            semantic_contract_digest: Some(manifest.capability_request_digest()?),
            source_digest: source_digest.clone(),
            lockfile_digests: lockfile_digests(&manifest, &workspace_root)?,
            adapter: manifest.build.adapter,
            adapter_version: prepared.adapter_version,
            platform,
            artifacts: prepared.artifacts,
            launch: prepared.launch,
            sandbox_profile: manifest.security.sandbox,
            runtime_network: manifest.requested_runtime_network()?,
            protocol_smoke: smoke,
            installed_at_unix: now_unix()?,
        };
        finish_install(
            &store,
            &staging,
            &mut guard,
            receipt,
            &manifest_digest,
            &source_digest,
            vec![
                "artifact".to_string(),
                "protocol_smoke".to_string(),
                "activate".to_string(),
            ],
            InstallOrigin::BuiltArtifact,
            request.control.as_ref(),
        )
    }

    /// Import a release-bundled, platform-specific Cargo skill after checking
    /// its immutable receipt, manifest identity, platform, size, and digest.
    /// No compiler or network access is used on this path.
    pub fn install_precompiled(
        &self,
        request: &PrecompiledInstallRequest,
    ) -> SkillSdkResult<InstallOutcome> {
        let _install_guard = install_process_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        emit_phase(request.control.as_ref(), "preflight")?;
        let workspace_root = fs::canonicalize(&request.workspace_root).map_err(|error| {
            SkillSdkError::new(
                "workspace_root_unavailable",
                format!("path={} error={error}", request.workspace_root.display()),
            )
            .phase("preflight")
        })?;
        let manifest_path = fs::canonicalize(&request.manifest_path).map_err(|error| {
            SkillSdkError::new(
                "manifest_read_failed",
                format!("path={} error={error}", request.manifest_path.display()),
            )
            .phase("manifest")
        })?;
        if !manifest_path.starts_with(&workspace_root) {
            return Err(SkillSdkError::new(
                "manifest_path_escape",
                manifest_path.display().to_string(),
            )
            .phase("preflight"));
        }
        let mut manifest = PackageManifest::load(&manifest_path)?;
        if manifest.build.adapter != BuildAdapter::Cargo {
            return Err(SkillSdkError::new(
                "precompiled_adapter_unsupported",
                format!("adapter={}", manifest.build.adapter.as_token()),
            )
            .phase("preflight"));
        }
        let platform = match request.target.as_deref() {
            Some(target) => HostPlatform::from_target(target)?,
            None => HostPlatform::current(),
        };
        manifest.validate_for_platform(&platform)?;
        let source_store = InstallReceiptStore::new(&request.precompiled_root);
        let pointer = source_store
            .current_pointer(&manifest.package.name)
            .map_err(|error| {
                SkillSdkError::new(
                    "precompiled_package_unavailable",
                    format!("skill={} detail={}", manifest.package.name, error.detail),
                )
                .phase("preflight")
            })?;
        let source_versions = source_store
            .skill_root(&manifest.package.name)?
            .join("versions");
        let canonical_versions = fs::canonicalize(&source_versions).map_err(|error| {
            SkillSdkError::new(
                "precompiled_package_unavailable",
                format!("path={} error={error}", source_versions.display()),
            )
            .phase("preflight")
        })?;
        let source_install = fs::canonicalize(source_versions.join(&pointer.install_dir))?;
        if !source_install.starts_with(&canonical_versions) {
            return Err(SkillSdkError::new(
                "precompiled_install_root_escape",
                source_install.display().to_string(),
            )
            .phase("precompiled_verify"));
        }
        let bundled_manifest = PackageManifest::load(&source_install.join("skill.toml"))?;
        let receipt: InstallReceipt =
            serde_json::from_slice(&fs::read(source_install.join("install-receipt.json"))?)?;
        receipt.validate()?;
        if receipt.schema_version == INSTALL_RECEIPT_SCHEMA_VERSION {
            manifest = manifest.into_current()?;
        }
        if receipt.digest()? != pointer.receipt_digest {
            return Err(SkillSdkError::new(
                "precompiled_receipt_digest_mismatch",
                format!("skill={}", manifest.package.name),
            )
            .phase("precompiled_verify"));
        }
        receipt.verifies_manifest(&bundled_manifest)?;
        receipt.verifies_manifest(&manifest).map_err(|error| {
            SkillSdkError::new("precompiled_manifest_mismatch", error.detail)
                .phase("precompiled_verify")
        })?;
        if receipt.platform.os != platform.os || receipt.platform.arch != platform.arch {
            return Err(SkillSdkError::new(
                "precompiled_platform_mismatch",
                format!(
                    "expected={}/{} actual={}/{}",
                    platform.os, platform.arch, receipt.platform.os, receipt.platform.arch
                ),
            )
            .phase("precompiled_verify"));
        }
        emit_phase(request.control.as_ref(), "precompiled_verify")?;
        fs::create_dir_all(&request.package_root)?;
        let destination_store = InstallReceiptStore::new(&request.package_root);
        let staging = destination_store.create_staging_dir(&manifest.package.name)?;
        let mut guard = StagingGuard::new(staging.clone());
        fs::write(staging.join("skill.toml"), manifest.to_toml_string()?)?;
        for artifact in &receipt.artifacts {
            let source = fs::canonicalize(source_install.join(&artifact.path))?;
            let metadata = fs::metadata(&source)?;
            if !source.starts_with(&source_install)
                || !metadata.is_file()
                || metadata.len() != artifact.size_bytes
                || digest_file(&source)? != artifact.sha256
            {
                return Err(SkillSdkError::new(
                    "precompiled_artifact_mismatch",
                    format!("path={}", artifact.path),
                )
                .phase("precompiled_verify"));
            }
            let destination = staging.join(&artifact.path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source, &destination)?;
            if artifact.executable {
                set_executable(&destination)?;
            }
        }
        fs::create_dir_all(staging.join(&receipt.launch.working_directory))?;
        emit_phase(request.control.as_ref(), "precompiled_copy")?;
        let manifest_digest = manifest.digest()?;
        let source_digest = receipt.source_digest.clone();
        finish_install(
            &destination_store,
            &staging,
            &mut guard,
            receipt,
            &manifest_digest,
            &source_digest,
            vec![
                "precompiled_verify".to_string(),
                "precompiled_copy".to_string(),
                "activate".to_string(),
            ],
            InstallOrigin::PlatformPrecompiled,
            request.control.as_ref(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_install(
    store: &InstallReceiptStore,
    staging: &Path,
    guard: &mut StagingGuard,
    receipt: InstallReceipt,
    manifest_digest: &str,
    source_digest: &str,
    phases: Vec<String>,
    origin: InstallOrigin,
    control: Option<&InstallControl>,
) -> SkillSdkResult<InstallOutcome> {
    emit_phase(control, "activate")?;
    store.write_receipt(staging, &receipt)?;
    let install_identity = receipt_install_identity(&receipt)?;
    let destination =
        store.version_dir(&receipt.skill_name, &receipt.version, &install_identity)?;
    fs::create_dir_all(destination.parent().ok_or_else(|| {
        SkillSdkError::new("install_parent_missing", destination.display().to_string())
    })?)?;
    let (receipt, reused) = if destination.exists() {
        let existing: InstallReceipt =
            serde_json::from_slice(&fs::read(destination.join("install-receipt.json"))?)?;
        existing.validate()?;
        if existing.manifest_digest != manifest_digest || existing.source_digest != source_digest {
            return Err(SkillSdkError::new(
                "install_identity_collision",
                destination.display().to_string(),
            )
            .phase("activate"));
        }
        (existing, true)
    } else {
        fs::rename(staging, &destination).map_err(|error| {
            SkillSdkError::new(
                "install_stage_commit_failed",
                format!(
                    "source={} destination={} error={error}",
                    staging.display(),
                    destination.display()
                ),
            )
            .phase("activate")
        })?;
        guard.disarm();
        (receipt, false)
    };
    store.activate(&destination, &receipt)?;
    Ok(InstallOutcome {
        skill_name: receipt.skill_name.clone(),
        version: receipt.version.clone(),
        install_root: fs::canonicalize(destination)?,
        receipt_digest: receipt.digest()?,
        adapter: receipt.adapter,
        origin,
        reused,
        phases,
    })
}

fn receipt_install_identity(receipt: &InstallReceipt) -> SkillSdkResult<String> {
    let identity = serde_json::json!({
        "manifest_digest": receipt.manifest_digest,
        "source_digest": receipt.source_digest,
        "lockfile_digests": receipt.lockfile_digests,
        "adapter": receipt.adapter,
        "adapter_version": receipt.adapter_version,
        "platform": receipt.platform,
        "artifacts": receipt.artifacts,
        "launch": receipt.launch,
        "sandbox_profile": receipt.sandbox_profile,
        "runtime_network": receipt.runtime_network,
    });
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&identity)?)))
}

fn install_process_slot() -> &'static Mutex<()> {
    static SLOT: OnceLock<Mutex<()>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(()))
}

fn protocol_smoke(
    manifest: &PackageManifest,
    prepared: &PreparedPackage,
    staging: &Path,
    workspace_root: &Path,
    platform: &HostPlatform,
    allow_network: bool,
    control: Option<&InstallControl>,
) -> SkillSdkResult<ProtocolSmokeReceipt> {
    let request_id = format!("skill-smoke-{}", Uuid::new_v4());
    if prepared.launch.launcher == crate::manifest::LauncherKind::HttpJson {
        if !allow_network {
            return Err(SkillSdkError::new(
                "protocol_smoke_network_approval_required",
                format!("skill={}", manifest.package.name),
            )
            .phase("protocol_smoke"));
        }
        if control.is_some_and(InstallControl::is_cancelled) {
            return Err(
                SkillSdkError::new("process_cancelled", "cancellation requested")
                    .phase("protocol_smoke"),
            );
        }
        let endpoint = prepared.launch.remote_endpoint.as_deref().ok_or_else(|| {
            SkillSdkError::new(
                "protocol_smoke_endpoint_missing",
                "http_json endpoint missing",
            )
            .phase("protocol_smoke")
        })?;
        let request = ProtocolRequest {
            request_id: request_id.clone(),
            args: manifest.run.smoke_args.clone(),
            context: Some(serde_json::json!({"protocol_smoke": true})),
            user_id: 0,
            chat_id: 0,
            user_key: None,
        };
        let timeout = Duration::from_secs(manifest.run.timeout_seconds.min(120));
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                SkillSdkError::new("protocol_smoke_http_client_failed", error.to_string())
                    .phase("protocol_smoke")
            })?;
        let response = client
            .post(endpoint)
            .json(&request)
            .send()
            .map_err(|error| {
                SkillSdkError::new("protocol_smoke_http_failed", error.to_string())
                    .phase("protocol_smoke")
            })?;
        if !response.status().is_success() {
            return Err(SkillSdkError::new(
                "protocol_smoke_http_status",
                format!("status={}", response.status().as_u16()),
            )
            .phase("protocol_smoke"));
        }
        if response
            .content_length()
            .is_some_and(|size| size > crate::protocol::MAX_PROTOCOL_LINE_BYTES as u64)
        {
            return Err(SkillSdkError::new(
                "protocol_response_oversized",
                "http_json response exceeds protocol limit",
            )
            .phase("protocol_smoke"));
        }
        let mut body = Vec::new();
        response
            .take((crate::protocol::MAX_PROTOCOL_LINE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|error| {
                SkillSdkError::new("protocol_smoke_http_read_failed", error.to_string())
                    .phase("protocol_smoke")
            })?;
        validate_response_line(&body, &request_id)?;
        if control.is_some_and(InstallControl::is_cancelled) {
            return Err(
                SkillSdkError::new("process_cancelled", "cancellation requested")
                    .phase("protocol_smoke"),
            );
        }
        return Ok(ProtocolSmokeReceipt {
            protocol: RUSTCLAW_JSONL_PROTOCOL.to_string(),
            passed: true,
            request_id,
            checked_at_unix: now_unix()?,
        });
    }
    let program = match prepared.launch.program_scope {
        LaunchProgramScope::Package => fs::canonicalize(staging.join(&prepared.launch.program))?,
        LaunchProgramScope::TrustedRuntime => fs::canonicalize(&prepared.launch.program)?,
    };
    let working_directory = fs::canonicalize(staging.join(&prepared.launch.working_directory))?;
    if !working_directory.starts_with(fs::canonicalize(staging)?) {
        return Err(SkillSdkError::new(
            "protocol_smoke_workdir_escape",
            working_directory.display().to_string(),
        )
        .phase("protocol_smoke"));
    }
    let host = HostPlatform::current();
    let (command_program, command_prefix) = if host.os == platform.os && host.arch == platform.arch
    {
        (program.clone(), Vec::new())
    } else {
        cross_protocol_emulator(platform, &program)?
    };
    let prepared_command = prepare_sandboxed_command(
        &command_program,
        &working_directory,
        &[],
        SandboxNetwork::Deny,
    )?;
    let mut command: Command = prepared_command.command;
    command.args(command_prefix);
    command.args(&prepared.launch.args);
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.envs(&prepared.launch.environment);
    for key in &prepared.launch.environment_allowlist {
        if sensitive_runtime_environment_name(key) {
            continue;
        }
        if key == "WORKSPACE_ROOT" {
            command.env(key, workspace_root);
        } else if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let request = ProtocolRequest {
        request_id: request_id.clone(),
        args: manifest.run.smoke_args.clone(),
        context: Some(serde_json::json!({"protocol_smoke": true})),
        user_id: 0,
        chat_id: 0,
        user_key: None,
    };
    let mut line = request.to_line()?.into_bytes();
    line.push(b'\n');
    let output = run_command_controlled(
        &mut command,
        Some(&line),
        Duration::from_secs(manifest.run.timeout_seconds.min(120)),
        "protocol_smoke",
        control.map(InstallControl::cancelled_flag),
    )?;
    if !output.status.success() {
        return Err(SkillSdkError::new(
            "protocol_smoke_process_failed",
            format!(
                "exit_code={:?} stderr={}",
                output.status.code(),
                crate::secret_scan::redact_diagnostics(&String::from_utf8_lossy(&output.stderr))
            ),
        )
        .phase("protocol_smoke"));
    }
    validate_response_line(&output.stdout, &request_id)?;
    Ok(ProtocolSmokeReceipt {
        protocol: RUSTCLAW_JSONL_PROTOCOL.to_string(),
        passed: true,
        request_id,
        checked_at_unix: now_unix()?,
    })
}

fn cross_protocol_emulator(
    platform: &HostPlatform,
    target_program: &Path,
) -> SkillSdkResult<(PathBuf, Vec<String>)> {
    if platform.os != "linux" {
        return Err(SkillSdkError::new(
            "cross_protocol_emulator_unsupported",
            format!("os={} arch={}", platform.os, platform.arch),
        )
        .phase("protocol_smoke"));
    }
    let (binary_names, sysroot) = match platform.arch.as_str() {
        "aarch64" => (
            &["qemu-aarch64-static", "qemu-aarch64"][..],
            Path::new("/usr/aarch64-linux-gnu"),
        ),
        "armv7" => (
            &["qemu-arm-static", "qemu-arm"][..],
            Path::new("/usr/arm-linux-gnueabihf"),
        ),
        "x86_64" => (
            &["qemu-x86_64-static", "qemu-x86_64"][..],
            Path::new("/usr/x86_64-linux-gnu"),
        ),
        _ => {
            return Err(SkillSdkError::new(
                "cross_protocol_emulator_unsupported",
                format!("os={} arch={}", platform.os, platform.arch),
            )
            .phase("protocol_smoke"))
        }
    };
    let emulator = binary_names
        .iter()
        .find_map(|name| find_on_path(name))
        .ok_or_else(|| {
            SkillSdkError::new(
                "cross_protocol_emulator_unavailable",
                format!("os={} arch={}", platform.os, platform.arch),
            )
            .phase("protocol_smoke")
        })?;
    let mut args = Vec::new();
    if sysroot.is_dir() {
        args.push("-L".to_string());
        args.push(sysroot.display().to_string());
    }
    args.push(target_program.display().to_string());
    Ok((emulator, args))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|root| root.join(name))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
}

fn sensitive_runtime_environment_name(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "CREDENTIAL",
        "API_KEY",
        "AUTH",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

fn set_executable(path: &Path) -> SkillSdkResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        fs::set_permissions(path, permissions)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(SkillSdkError::new(
            "adopt_platform_unsupported",
            format!("platform={}", std::env::consts::OS),
        ))
    }
}

fn emit_phase(control: Option<&InstallControl>, phase: &str) -> SkillSdkResult<()> {
    match control {
        Some(control) => control.phase(phase),
        None => Ok(()),
    }
}

fn lockfile_digests(
    manifest: &PackageManifest,
    workspace_root: &Path,
) -> SkillSdkResult<BTreeMap<String, String>> {
    let mut digests = BTreeMap::new();
    let Some(relative) = manifest.build.lockfile.as_deref() else {
        return Ok(digests);
    };
    let source_root = fs::canonicalize(workspace_root.join(&manifest.build.source_root))?;
    let lockfile = fs::canonicalize(source_root.join(relative))?;
    if !lockfile.starts_with(&source_root) || !lockfile.is_file() {
        return Err(
            SkillSdkError::new("lockfile_path_escape", lockfile.display().to_string())
                .phase("dependencies"),
        );
    }
    digests.insert(relative.to_string(), digest_file(&lockfile)?);
    Ok(digests)
}

fn now_unix() -> SkillSdkResult<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| SkillSdkError::new("system_clock_invalid", error.to_string()))
}

struct StagingGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
#[path = "installer_tests.rs"]
mod tests;
