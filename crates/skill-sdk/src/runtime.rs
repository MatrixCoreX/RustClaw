use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{
    BuildAdapter, ExecutionProfile, LauncherKind, PackageManifest, SandboxProfile,
};
use crate::receipt::{
    digest_file, ArtifactReceipt, InstallReceipt, InstallReceiptStore, LaunchProgramScope,
};
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub progress_frames: bool,
    #[serde(default)]
    pub execution_profile: ExecutionProfile,
    pub sandbox_profile: SandboxProfile,
    pub runtime_network: bool,
    pub install_root: PathBuf,
    pub manifest_digest: String,
    pub receipt_digest: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkillVersionPin {
    pub skill_name: String,
    pub version: String,
    pub adapter: BuildAdapter,
    pub progress_frames: bool,
    pub execution_profile: ExecutionProfile,
    pub sandbox_profile: SandboxProfile,
    pub environment_allowlist: Vec<String>,
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
        self.resolve_install_directory(
            skill_name,
            &pointer.install_dir,
            Some(&pointer.version),
            Some(&pointer.receipt_digest),
            None,
        )
    }

    pub fn pin_current(&self, skill_name: &str) -> SkillSdkResult<SkillVersionPin> {
        let pointer = self.store.current_pointer(skill_name).map_err(|error| {
            SkillSdkError::new("launch_receipt_missing", error.to_string()).phase("resolve")
        })?;
        self.pin_install_directory(
            skill_name,
            &pointer.install_dir,
            Some(&pointer.version),
            Some(&pointer.receipt_digest),
            None,
        )
    }

    /// Inspect the active package for control-plane display without hashing
    /// every installed artifact.
    ///
    /// This validates the pointer, receipt, manifest, confined paths, and
    /// artifact sizes. It intentionally is not an execution authorization:
    /// callers that launch a skill must still use `resolve`/`resolve_pinned`,
    /// which verify every artifact digest immediately before execution.
    pub fn inspect_current(&self, skill_name: &str) -> SkillSdkResult<SkillVersionPin> {
        let pointer = self.store.current_pointer(skill_name)?;
        let (canonical_install, manifest, receipt, receipt_digest) = self
            .verified_receipt_metadata(
                skill_name,
                &pointer.install_dir,
                Some(&pointer.version),
                Some(&pointer.receipt_digest),
                None,
            )?;
        match receipt.launch.program_scope {
            LaunchProgramScope::Package => {
                verify_launch_artifact_metadata(
                    &canonical_install,
                    &receipt,
                    &receipt.launch.program,
                )?;
            }
            LaunchProgramScope::TrustedRuntime => {
                let program = fs::canonicalize(&receipt.launch.program)?;
                if !program.is_file() {
                    return Err(SkillSdkError::new(
                        "launch_runtime_missing",
                        format!("program={}", program.display()),
                    ));
                }
            }
        }
        confined_existing_path(&canonical_install, &receipt.launch.working_directory, false)?;
        for argument in &receipt.launch.args {
            if receipt
                .artifacts
                .iter()
                .any(|artifact| artifact.path == *argument)
            {
                verify_launch_artifact_metadata(&canonical_install, &receipt, argument)?;
            }
        }
        Ok(SkillVersionPin {
            skill_name: receipt.skill_name,
            version: receipt.version,
            adapter: receipt.adapter,
            progress_frames: manifest.run.progress_frames,
            execution_profile: manifest.run.execution_profile,
            sandbox_profile: receipt.sandbox_profile,
            environment_allowlist: receipt.launch.environment_allowlist,
            install_root: canonical_install,
            manifest_digest: manifest.digest()?,
            receipt_digest,
        })
    }

    pub fn pin_exact(
        &self,
        skill_name: &str,
        expected_version: &str,
        expected_manifest_digest: &str,
        expected_receipt_digest: &str,
    ) -> SkillSdkResult<SkillVersionPin> {
        let install_dir = self.find_pinned_install_dir(
            skill_name,
            expected_version,
            expected_manifest_digest,
            expected_receipt_digest,
        )?;
        self.pin_install_directory(
            skill_name,
            &install_dir,
            Some(expected_version),
            Some(expected_receipt_digest),
            Some(expected_manifest_digest),
        )
    }

    /// Resolve an immutable installed version without consulting `current.json`.
    ///
    /// The caller pins these values when a capability step starts. This closes
    /// the update/uninstall TOCTOU window where a runner could otherwise select
    /// a different version after the host had already authorized the step.
    pub fn resolve_pinned(
        &self,
        skill_name: &str,
        expected_version: &str,
        expected_manifest_digest: &str,
        expected_receipt_digest: &str,
    ) -> SkillSdkResult<SkillLaunchSpec> {
        let install_dir = self.find_pinned_install_dir(
            skill_name,
            expected_version,
            expected_manifest_digest,
            expected_receipt_digest,
        )?;
        self.resolve_install_directory(
            skill_name,
            &install_dir,
            Some(expected_version),
            Some(expected_receipt_digest),
            Some(expected_manifest_digest),
        )
    }

    fn find_pinned_install_dir(
        &self,
        skill_name: &str,
        expected_version: &str,
        expected_manifest_digest: &str,
        expected_receipt_digest: &str,
    ) -> SkillSdkResult<String> {
        let versions_root = self.store.skill_root(skill_name)?.join("versions");
        let canonical_versions = fs::canonicalize(&versions_root)?;
        let mut matches = Vec::new();
        for entry in fs::read_dir(&canonical_versions)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let receipt_path = entry.path().join("install-receipt.json");
            if !receipt_path.is_file() {
                continue;
            }
            let receipt: InstallReceipt = serde_json::from_slice(&fs::read(receipt_path)?)?;
            receipt.validate()?;
            if receipt.skill_name == skill_name
                && receipt.version == expected_version
                && receipt.manifest_digest == expected_manifest_digest
                && receipt.digest()? == expected_receipt_digest
            {
                matches.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        match matches.as_slice() {
            [install_dir] => Ok(install_dir.clone()),
            [] => Err(SkillSdkError::new(
                "launch_pinned_version_missing",
                format!("skill={skill_name} version={expected_version}"),
            )),
            _ => Err(SkillSdkError::new(
                "launch_pinned_version_ambiguous",
                format!("skill={skill_name} matches={}", matches.len()),
            )),
        }
    }

    fn pin_install_directory(
        &self,
        skill_name: &str,
        install_dir: &str,
        expected_version: Option<&str>,
        expected_receipt_digest: Option<&str>,
        expected_manifest_digest: Option<&str>,
    ) -> SkillSdkResult<SkillVersionPin> {
        let (canonical_install, manifest, receipt, receipt_digest) = self
            .verified_receipt_metadata(
                skill_name,
                install_dir,
                expected_version,
                expected_receipt_digest,
                expected_manifest_digest,
            )?;
        Ok(SkillVersionPin {
            skill_name: receipt.skill_name,
            version: receipt.version,
            adapter: receipt.adapter,
            progress_frames: manifest.run.progress_frames,
            execution_profile: manifest.run.execution_profile,
            sandbox_profile: receipt.sandbox_profile,
            environment_allowlist: receipt.launch.environment_allowlist,
            install_root: canonical_install,
            manifest_digest: manifest.digest()?,
            receipt_digest,
        })
    }

    fn resolve_install_directory(
        &self,
        skill_name: &str,
        install_dir: &str,
        expected_version: Option<&str>,
        expected_receipt_digest: Option<&str>,
        expected_manifest_digest: Option<&str>,
    ) -> SkillSdkResult<SkillLaunchSpec> {
        let (canonical_install, manifest, receipt, receipt_digest) = self
            .verified_receipt_metadata(
                skill_name,
                install_dir,
                expected_version,
                expected_receipt_digest,
                expected_manifest_digest,
            )?;
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
            progress_frames: manifest.run.progress_frames,
            execution_profile: manifest.run.execution_profile,
            sandbox_profile: receipt.sandbox_profile,
            runtime_network: receipt.runtime_network,
            install_root: canonical_install,
            manifest_digest: receipt.manifest_digest,
            receipt_digest,
        };
        launch.validate()?;
        Ok(launch)
    }

    fn verified_receipt_metadata(
        &self,
        skill_name: &str,
        install_dir: &str,
        expected_version: Option<&str>,
        expected_receipt_digest: Option<&str>,
        expected_manifest_digest: Option<&str>,
    ) -> SkillSdkResult<(PathBuf, PackageManifest, InstallReceipt, String)> {
        let skill_root = self.store.skill_root(skill_name)?;
        let versions_root = skill_root.join("versions");
        let install_root = versions_root.join(install_dir);
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
        if receipt.skill_name != skill_name {
            return Err(SkillSdkError::new(
                "launch_skill_identity_mismatch",
                format!("expected={skill_name} actual={}", receipt.skill_name),
            ));
        }
        if expected_version.is_some_and(|expected| receipt.version != expected) {
            return Err(SkillSdkError::new(
                "launch_version_mismatch",
                format!(
                    "expected={} actual={}",
                    expected_version.unwrap_or_default(),
                    receipt.version
                ),
            ));
        }
        if expected_manifest_digest.is_some_and(|expected| receipt.manifest_digest != expected) {
            return Err(SkillSdkError::new(
                "launch_manifest_digest_mismatch",
                format!(
                    "expected={} actual={}",
                    expected_manifest_digest.unwrap_or_default(),
                    receipt.manifest_digest
                ),
            ));
        }
        if expected_receipt_digest.is_some_and(|expected| receipt_digest != expected) {
            return Err(SkillSdkError::new(
                "launch_receipt_digest_mismatch",
                format!(
                    "expected={} actual={receipt_digest}",
                    expected_receipt_digest.unwrap_or_default()
                ),
            ));
        }
        Ok((canonical_install, manifest, receipt, receipt_digest))
    }
}

fn verify_artifacts(install_root: &Path, receipt: &InstallReceipt) -> SkillSdkResult<()> {
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8)
        .min(receipt.artifacts.len());
    if worker_count <= 1 || receipt.artifacts.len() < 32 {
        return receipt
            .artifacts
            .iter()
            .try_for_each(|artifact| verify_artifact(install_root, artifact));
    }

    // Balance both byte hashing and per-file open/stat overhead. The receipt
    // remains fully verified; this only uses the host's available CPU and I/O
    // concurrency instead of serially walking large private environments.
    let mut ordered = receipt.artifacts.iter().collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|artifact| std::cmp::Reverse(artifact.size_bytes));
    let mut buckets = vec![Vec::new(); worker_count];
    let mut loads = vec![0_u64; worker_count];
    for artifact in ordered {
        let bucket = loads
            .iter()
            .enumerate()
            .min_by_key(|(_, load)| *load)
            .map(|(index, _)| index)
            .unwrap_or(0);
        buckets[bucket].push(artifact);
        loads[bucket] = loads[bucket].saturating_add(artifact.size_bytes.saturating_add(64 * 1024));
    }

    std::thread::scope(|scope| {
        let workers = buckets
            .into_iter()
            .map(|bucket| {
                scope.spawn(move || {
                    bucket
                        .into_iter()
                        .try_for_each(|artifact| verify_artifact(install_root, artifact))
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            match worker.join() {
                Ok(result) => result?,
                Err(_) => {
                    return Err(SkillSdkError::new(
                        "launch_artifact_verify_worker_panicked",
                        "artifact verification worker panicked",
                    ));
                }
            }
        }
        Ok(())
    })
}

fn verify_launch_artifact_metadata(
    install_root: &Path,
    receipt: &InstallReceipt,
    relative: &str,
) -> SkillSdkResult<()> {
    let path = confined_existing_path(install_root, relative, true)?;
    let artifact = receipt
        .artifacts
        .iter()
        .find(|artifact| artifact.path == relative)
        .ok_or_else(|| {
            SkillSdkError::new(
                "launch_artifact_receipt_missing",
                format!("path={relative}"),
            )
        })?;
    if fs::metadata(&path)?.len() != artifact.size_bytes {
        return Err(SkillSdkError::new(
            "launch_artifact_metadata_mismatch",
            format!("path={}", path.display()),
        ));
    }
    Ok(())
}

fn verify_artifact(install_root: &Path, artifact: &ArtifactReceipt) -> SkillSdkResult<()> {
    let path = confined_existing_path(install_root, &artifact.path, true)?;
    let metadata = fs::metadata(&path)?;
    if metadata.len() != artifact.size_bytes || digest_file(&path)? != artifact.sha256 {
        return Err(SkillSdkError::new(
            "launch_artifact_digest_mismatch",
            format!("path={}", path.display()),
        ));
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
