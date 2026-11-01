use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::sandbox_private::DirectoryExclusions;
use crate::ProcessInvocation;
use crate::{SkillSdkError, SkillSdkResult};

/// Set by the trusted runtime when the skill runner and all of its descendants
/// already execute inside the capability-scoped process sandbox.
pub const PARENT_SANDBOX_BACKEND_ENV: &str = "APP_PARENT_SANDBOX_BACKEND";
/// Set only by the trusted runtime so a read-only installed skill can write to
/// its declared private storage without gaining write access to the workspace.
pub const SKILL_STORAGE_WRITABLE_DIRECTORY_ENV: &str = "APP_SKILL_STORAGE_WRITABLE_DIRECTORY";
/// Projection of the current private-filesystem recipe and exact exclusions
/// enforced by a trusted outer sandbox.
/// This digest is a consistency check, not authentication against a host admin.
pub const PARENT_PRIVATE_DIRECTORIES_DIGEST_ENV: &str = "APP_PARENT_PRIVATE_DIRECTORIES_DIGEST";

#[cfg(target_os = "linux")]
const PRIVATE_DIRECTORY_RECIPE: &[u8] = b"bubblewrap-ancestor-anchors-v2";
#[cfg(target_os = "macos")]
const PRIVATE_DIRECTORY_RECIPE: &[u8] = b"seatbelt-subpath-v1";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const PRIVATE_DIRECTORY_RECIPE: &[u8] = b"unsupported-v1";

pub fn private_directories_digest(directories: &[PathBuf]) -> SkillSdkResult<String> {
    if directories.len() > 32 {
        return Err(SkillSdkError::new(
            "sandbox_private_directories_limit",
            "sandbox_private_directories_limit",
        ));
    }
    let bytes = serde_json::to_vec(directories).map_err(|_| {
        SkillSdkError::new(
            "sandbox_private_directory_invalid",
            "sandbox_private_directory_invalid",
        )
    })?;
    let mut digest = Sha256::new();
    digest.update(b"sandbox-private-policy-v2\0");
    digest.update(PRIVATE_DIRECTORY_RECIPE);
    digest.update([0]);
    digest.update(bytes);
    Ok(hex::encode(digest.finalize()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxNetwork {
    Deny,
    Allow,
}

pub struct PreparedSandboxInvocation {
    pub invocation: ProcessInvocation,
    pub backend: &'static str,
    working_directory: Option<PathBuf>,
}

impl PreparedSandboxInvocation {
    pub fn into_command(self) -> Command {
        let mut command = self.invocation.into_command();
        if let Some(directory) = self.working_directory {
            command.current_dir(directory);
        }
        command
    }
}

pub fn prepare_sandboxed_invocation(
    program: &Path,
    execution_root: &Path,
    writable_paths: &[PathBuf],
    network: SandboxNetwork,
    host_private_directories: &[PathBuf],
) -> SkillSdkResult<PreparedSandboxInvocation> {
    if !program.is_absolute() || !program.is_file() {
        return Err(
            SkillSdkError::new("sandbox_program_invalid", program.display().to_string())
                .phase("preflight"),
        );
    }
    let execution_root = canonical_directory(execution_root)?;
    let writable_paths = writable_paths
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<SkillSdkResult<Vec<_>>>()?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let exclusions = DirectoryExclusions::admit(host_private_directories, &execution_root)
        .map_err(private_directory_failure)?;
    #[cfg(target_os = "linux")]
    {
        prepare_bubblewrap(
            program,
            &execution_root,
            &writable_paths,
            network,
            &exclusions,
        )
    }
    #[cfg(target_os = "macos")]
    {
        prepare_macos_seatbelt(
            program,
            &execution_root,
            &writable_paths,
            network,
            &exclusions,
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (
            execution_root,
            writable_paths,
            network,
            host_private_directories,
        );
        Err(SkillSdkError::new(
            "sandbox_platform_unsupported",
            format!("platform={}", std::env::consts::OS),
        )
        .phase("preflight"))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_directory_failure(error_code: &'static str) -> SkillSdkError {
    SkillSdkError::new(error_code, error_code).phase("preflight")
}

fn canonical_directory(path: &Path) -> SkillSdkResult<PathBuf> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        SkillSdkError::new(
            "sandbox_directory_unavailable",
            format!("path={} error={error}", path.display()),
        )
        .phase("preflight")
    })?;
    if !canonical.is_dir() {
        return Err(SkillSdkError::new(
            "sandbox_path_not_directory",
            canonical.display().to_string(),
        )
        .phase("preflight"));
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn prepare_bubblewrap(
    program: &Path,
    execution_root: &Path,
    writable_paths: &[PathBuf],
    network: SandboxNetwork,
    exclusions: &DirectoryExclusions,
) -> SkillSdkResult<PreparedSandboxInvocation> {
    let backend = [Path::new("/usr/bin/bwrap"), Path::new("/bin/bwrap")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            SkillSdkError::new("sandbox_backend_unavailable", "bubblewrap not found")
                .phase("preflight")
        })?;
    let mut command = ProcessInvocation::new(backend);
    command.arg("--die-with-parent").arg("--new-session");
    if !exclusions.is_empty() {
        command.arg("--unshare-user");
    }
    command
        .arg("--unshare-pid")
        .arg("--unshare-ipc")
        .arg("--unshare-uts")
        .arg("--ro-bind")
        .arg("/")
        .arg("/")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev");
    let host_tmp = Path::new("/tmp");
    if !execution_root.starts_with(host_tmp)
        && !writable_paths.iter().any(|path| path.starts_with(host_tmp))
    {
        command.arg("--tmpfs").arg(host_tmp);
    }
    if network == SandboxNetwork::Deny {
        command.arg("--unshare-net");
    }
    for path in writable_paths {
        command.arg("--bind").arg(path).arg(path);
    }
    let mappings = writable_paths
        .iter()
        .map(|path| (path.clone(), path.clone()))
        .collect::<Vec<_>>();
    for (source, target) in exclusions
        .mount_anchors(&mappings)
        .map_err(private_directory_failure)?
    {
        command.arg("--bind").arg(source).arg(target);
    }
    for target in exclusions
        .mount_targets(&mappings)
        .map_err(private_directory_failure)?
    {
        command
            .arg("--tmpfs")
            .arg(&target)
            .arg("--remount-ro")
            .arg(&target);
    }
    command
        .arg("--chdir")
        .arg(execution_root)
        .arg("--")
        .arg(program);
    Ok(PreparedSandboxInvocation {
        invocation: command,
        backend: "bubblewrap",
        working_directory: None,
    })
}

#[cfg(target_os = "macos")]
fn prepare_macos_seatbelt(
    program: &Path,
    execution_root: &Path,
    writable_paths: &[PathBuf],
    network: SandboxNetwork,
    exclusions: &DirectoryExclusions,
) -> SkillSdkResult<PreparedSandboxInvocation> {
    let backend = Path::new("/usr/bin/sandbox-exec");
    if !backend.is_file() {
        return Err(SkillSdkError::new(
            "sandbox_backend_unavailable",
            "macOS sandbox-exec not found",
        )
        .phase("preflight"));
    }
    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    if network == SandboxNetwork::Allow {
        profile.push_str("(allow network*)\n");
    } else {
        profile.push_str("(deny network*)\n");
    }
    let mut macos_writable_paths = writable_paths.to_vec();
    if let Ok(temp_directory) = std::fs::canonicalize(std::env::temp_dir()) {
        if temp_directory.is_dir() && !macos_writable_paths.contains(&temp_directory) {
            macos_writable_paths.push(temp_directory);
        }
    }
    for path in &macos_writable_paths {
        exclusions
            .check_writable(path)
            .map_err(private_directory_failure)?;
        let escaped = crate::sandbox_private::seatbelt_path_literal(path)
            .map_err(|code| SkillSdkError::new(code, path.display().to_string()))?;
        profile.push_str(&format!("(allow file-write* (subpath \"{escaped}\"))\n"));
    }
    profile.push_str(
        &exclusions
            .seatbelt_rules()
            .map_err(private_directory_failure)?,
    );
    let mut command = ProcessInvocation::new(backend);
    command.arg("-p").arg(profile).arg(program);
    Ok(PreparedSandboxInvocation {
        invocation: command,
        backend: "macos_seatbelt",
        working_directory: Some(execution_root.to_path_buf()),
    })
}

#[cfg(all(test, target_os = "macos"))]
#[path = "sandbox_tests.rs"]
mod tests;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "sandbox_visibility_tests.rs"]
mod visibility_tests;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "sandbox_invocation_tests.rs"]
mod invocation_tests;
