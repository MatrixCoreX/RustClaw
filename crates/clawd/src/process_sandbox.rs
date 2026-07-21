use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use claw_core::config::{ToolSandboxBackend, ToolSandboxMode};
use serde::Serialize;
use tokio::process::Command;

#[cfg(target_os = "linux")]
const BUBBLEWRAP_BACKEND: &str = "bubblewrap";
#[cfg(target_os = "macos")]
const MACOS_SEATBELT_BACKEND: &str = "macos_seatbelt";
const REMOTE_CONTAINER_BACKEND: &str = "remote_container";
const DIRECT_BACKEND: &str = "direct";
#[cfg(target_os = "linux")]
const INTERNAL_WRITABLE_ROOT: &str = "/run/rustclaw-writable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessNetworkPolicy {
    Deny,
    Inherit,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessSandboxRequest<'a> {
    pub(crate) mode: ToolSandboxMode,
    pub(crate) backend: ToolSandboxBackend,
    pub(crate) workspace_root: &'a Path,
    pub(crate) execution_root: &'a Path,
    pub(crate) network: ProcessNetworkPolicy,
    pub(crate) additional_writable_paths: &'a [PathBuf],
}

pub(crate) struct PreparedProcessCommand {
    pub(crate) command: Command,
    pub(crate) backend: &'static str,
    pub(crate) additional_writable_targets: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SandboxControlDiagnostics {
    pub(crate) filesystem: &'static str,
    pub(crate) network: &'static str,
    pub(crate) process: &'static str,
    pub(crate) credential: &'static str,
    pub(crate) resource: &'static str,
    pub(crate) environment: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SandboxBackendDiagnostics {
    pub(crate) schema_version: u8,
    pub(crate) requested_backend: &'static str,
    pub(crate) resolved_backend: &'static str,
    pub(crate) platform: &'static str,
    pub(crate) available: bool,
    pub(crate) fail_closed: bool,
    pub(crate) reason_code: Option<&'static str>,
    pub(crate) controls: SandboxControlDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessLifetime {
    ParentBound,
    DurableAsync,
}

trait ProcessSandboxBackend {
    fn token(&self) -> &'static str;
    fn available(&self) -> bool;
    fn controls(&self, network: ProcessNetworkPolicy) -> SandboxControlDiagnostics;
    fn prepare(
        &self,
        program: &OsStr,
        request: ProcessSandboxRequest<'_>,
        lifetime: ProcessLifetime,
    ) -> Result<PreparedProcessCommand, &'static str>;
}

#[cfg(target_os = "linux")]
struct BubblewrapBackend;
#[cfg(target_os = "macos")]
struct MacosSeatbeltBackend;
struct RemoteContainerBackend;

enum ResolvedBackend {
    #[cfg(target_os = "linux")]
    Bubblewrap(BubblewrapBackend),
    #[cfg(target_os = "macos")]
    MacosSeatbelt(MacosSeatbeltBackend),
    RemoteContainer(RemoteContainerBackend),
}

impl ResolvedBackend {
    fn driver(&self) -> &dyn ProcessSandboxBackend {
        match self {
            #[cfg(target_os = "linux")]
            Self::Bubblewrap(backend) => backend,
            #[cfg(target_os = "macos")]
            Self::MacosSeatbelt(backend) => backend,
            Self::RemoteContainer(backend) => backend,
        }
    }
}

pub(crate) fn sandbox_backend_diagnostics(
    requested: ToolSandboxBackend,
    mode: ToolSandboxMode,
    network: ProcessNetworkPolicy,
) -> SandboxBackendDiagnostics {
    if mode == ToolSandboxMode::DangerFull {
        return SandboxBackendDiagnostics {
            schema_version: 1,
            requested_backend: requested.as_token(),
            resolved_backend: DIRECT_BACKEND,
            platform: std::env::consts::OS,
            available: true,
            fail_closed: false,
            reason_code: None,
            controls: inherited_controls(network),
        };
    }
    let resolved = match resolve_backend(requested) {
        Ok(resolved) => resolved,
        Err(reason_code) => {
            return SandboxBackendDiagnostics {
                schema_version: 1,
                requested_backend: requested.as_token(),
                resolved_backend: "unsupported",
                platform: std::env::consts::OS,
                available: false,
                fail_closed: true,
                reason_code: Some(reason_code),
                controls: unavailable_controls(),
            };
        }
    };
    let driver = resolved.driver();
    let available = driver.available();
    SandboxBackendDiagnostics {
        schema_version: 1,
        requested_backend: requested.as_token(),
        resolved_backend: driver.token(),
        platform: std::env::consts::OS,
        available,
        fail_closed: true,
        reason_code: (!available).then_some(match requested {
            ToolSandboxBackend::RemoteContainer => "sandbox_remote_backend_not_configured",
            _ => "sandbox_backend_unavailable",
        }),
        controls: driver.controls(network),
    }
}

pub(crate) fn prepare_process_command(
    program: impl AsRef<OsStr>,
    request: ProcessSandboxRequest<'_>,
) -> Result<PreparedProcessCommand, &'static str> {
    prepare_process_command_for_lifetime(program, request, ProcessLifetime::ParentBound)
}

pub(crate) fn prepare_durable_process_command(
    program: impl AsRef<OsStr>,
    request: ProcessSandboxRequest<'_>,
) -> Result<PreparedProcessCommand, &'static str> {
    prepare_process_command_for_lifetime(program, request, ProcessLifetime::DurableAsync)
}

fn prepare_process_command_for_lifetime(
    program: impl AsRef<OsStr>,
    request: ProcessSandboxRequest<'_>,
    lifetime: ProcessLifetime,
) -> Result<PreparedProcessCommand, &'static str> {
    if request.mode == ToolSandboxMode::DangerFull {
        return Ok(PreparedProcessCommand {
            command: Command::new(program),
            backend: DIRECT_BACKEND,
            additional_writable_targets: request.additional_writable_paths.to_vec(),
        });
    }

    let resolved = resolve_backend(request.backend)?;
    let driver = resolved.driver();
    if !driver.available() {
        return Err(match request.backend {
            ToolSandboxBackend::RemoteContainer => "sandbox_remote_backend_not_configured",
            _ => "sandbox_backend_unavailable",
        });
    }
    driver.prepare(program.as_ref(), request, lifetime)
}

fn resolve_backend(requested: ToolSandboxBackend) -> Result<ResolvedBackend, &'static str> {
    match requested {
        ToolSandboxBackend::Auto => {
            #[cfg(target_os = "linux")]
            {
                Ok(ResolvedBackend::Bubblewrap(BubblewrapBackend))
            }
            #[cfg(target_os = "macos")]
            {
                Ok(ResolvedBackend::MacosSeatbelt(MacosSeatbeltBackend))
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                Err("sandbox_backend_unsupported_platform")
            }
        }
        ToolSandboxBackend::Bubblewrap => {
            #[cfg(target_os = "linux")]
            {
                Ok(ResolvedBackend::Bubblewrap(BubblewrapBackend))
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err("sandbox_backend_unsupported_platform")
            }
        }
        ToolSandboxBackend::MacosSeatbelt => {
            #[cfg(target_os = "macos")]
            {
                Ok(ResolvedBackend::MacosSeatbelt(MacosSeatbeltBackend))
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("sandbox_backend_unsupported_platform")
            }
        }
        ToolSandboxBackend::RemoteContainer => {
            Ok(ResolvedBackend::RemoteContainer(RemoteContainerBackend))
        }
    }
}

#[cfg(target_os = "linux")]
impl ProcessSandboxBackend for BubblewrapBackend {
    fn token(&self) -> &'static str {
        BUBBLEWRAP_BACKEND
    }

    fn available(&self) -> bool {
        find_bubblewrap().is_some()
    }

    fn controls(&self, network: ProcessNetworkPolicy) -> SandboxControlDiagnostics {
        SandboxControlDiagnostics {
            filesystem: "namespace_write_scoped",
            network: network_control(network),
            process: "pid_ipc_uts_namespaces",
            credential: "caller_environment_and_filesystem_policy",
            resource: "caller_managed",
            environment: "caller_managed",
        }
    }

    fn prepare(
        &self,
        program: &OsStr,
        request: ProcessSandboxRequest<'_>,
        lifetime: ProcessLifetime,
    ) -> Result<PreparedProcessCommand, &'static str> {
        prepare_bubblewrap(program, request, lifetime)
    }
}

#[cfg(target_os = "macos")]
impl ProcessSandboxBackend for MacosSeatbeltBackend {
    fn token(&self) -> &'static str {
        MACOS_SEATBELT_BACKEND
    }

    fn available(&self) -> bool {
        seatbelt_path().is_some()
    }

    fn controls(&self, network: ProcessNetworkPolicy) -> SandboxControlDiagnostics {
        SandboxControlDiagnostics {
            filesystem: "seatbelt_write_scoped",
            network: network_control(network),
            process: "seatbelt_policy",
            credential: "caller_environment_and_filesystem_policy",
            resource: "caller_managed",
            environment: "caller_managed",
        }
    }

    fn prepare(
        &self,
        program: &OsStr,
        request: ProcessSandboxRequest<'_>,
        lifetime: ProcessLifetime,
    ) -> Result<PreparedProcessCommand, &'static str> {
        prepare_macos_seatbelt(program, request, lifetime)
    }
}

impl ProcessSandboxBackend for RemoteContainerBackend {
    fn token(&self) -> &'static str {
        REMOTE_CONTAINER_BACKEND
    }

    fn available(&self) -> bool {
        false
    }

    fn controls(&self, _network: ProcessNetworkPolicy) -> SandboxControlDiagnostics {
        unavailable_controls()
    }

    fn prepare(
        &self,
        _program: &OsStr,
        _request: ProcessSandboxRequest<'_>,
        _lifetime: ProcessLifetime,
    ) -> Result<PreparedProcessCommand, &'static str> {
        Err("sandbox_remote_backend_not_configured")
    }
}

#[cfg(target_os = "linux")]
fn prepare_bubblewrap(
    program: &OsStr,
    request: ProcessSandboxRequest<'_>,
    lifetime: ProcessLifetime,
) -> Result<PreparedProcessCommand, &'static str> {
    let backend = find_bubblewrap().ok_or("sandbox_backend_unavailable")?;
    let (workspace_root, execution_root) = canonical_roots(request)?;
    let mut command = Command::new(backend);
    if lifetime == ProcessLifetime::ParentBound {
        command.arg("--die-with-parent");
    }
    command
        .arg("--new-session")
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
    if request.network == ProcessNetworkPolicy::Deny {
        command.arg("--unshare-net");
    }
    command.arg("--tmpfs").arg("/tmp");
    if sandbox_writes_workspace(request.mode) {
        command
            .arg("--bind")
            .arg(&workspace_root)
            .arg(&workspace_root);
    } else {
        command
            .arg("--ro-bind")
            .arg(&workspace_root)
            .arg(&workspace_root);
    }
    let mut additional_writable_targets = Vec::new();
    if !request.additional_writable_paths.is_empty() {
        command
            .arg("--tmpfs")
            .arg("/run")
            .arg("--dir")
            .arg(INTERNAL_WRITABLE_ROOT);
    }
    for (index, path) in request.additional_writable_paths.iter().enumerate() {
        let source = canonical_directory(path)?;
        let target = Path::new(INTERNAL_WRITABLE_ROOT).join(index.to_string());
        command
            .arg("--dir")
            .arg(&target)
            .arg("--bind")
            .arg(&source)
            .arg(&target);
        additional_writable_targets.push(target);
    }
    command
        .arg("--chdir")
        .arg(&execution_root)
        .arg("--")
        .arg(program);
    Ok(PreparedProcessCommand {
        command,
        backend: BUBBLEWRAP_BACKEND,
        additional_writable_targets,
    })
}

#[cfg(target_os = "macos")]
fn prepare_macos_seatbelt(
    program: &OsStr,
    request: ProcessSandboxRequest<'_>,
    _lifetime: ProcessLifetime,
) -> Result<PreparedProcessCommand, &'static str> {
    let backend = seatbelt_path().ok_or("sandbox_backend_unavailable")?;
    let (workspace_root, _execution_root) = canonical_roots(request)?;
    let mut writable_paths = request
        .additional_writable_paths
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<Result<Vec<_>, _>>()?;
    if sandbox_writes_workspace(request.mode) {
        writable_paths.push(workspace_root);
    }
    for temp in [Path::new("/private/tmp"), Path::new("/tmp")] {
        if let Ok(temp) = canonical_directory(temp) {
            if !writable_paths.contains(&temp) {
                writable_paths.push(temp);
            }
        }
    }
    let profile = build_macos_seatbelt_profile(request.network, &writable_paths)?;
    let mut command = Command::new(backend);
    command.arg("-p").arg(profile).arg(program);
    Ok(PreparedProcessCommand {
        command,
        backend: MACOS_SEATBELT_BACKEND,
        additional_writable_targets: request
            .additional_writable_paths
            .iter()
            .map(|path| canonical_directory(path))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn canonical_roots(request: ProcessSandboxRequest<'_>) -> Result<(PathBuf, PathBuf), &'static str> {
    let workspace_root = canonical_directory(request.workspace_root)?;
    let execution_root = canonical_directory(request.execution_root)?;
    if sandbox_writes_workspace(request.mode) && !execution_root.starts_with(&workspace_root) {
        return Err("sandbox_execution_root_outside_workspace");
    }
    Ok((workspace_root, execution_root))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn canonical_directory(path: &Path) -> Result<PathBuf, &'static str> {
    let canonical = path
        .canonicalize()
        .map_err(|_| "sandbox_directory_unavailable")?;
    if !canonical.is_dir() {
        return Err("sandbox_path_not_directory");
    }
    Ok(canonical)
}

fn sandbox_writes_workspace(mode: ToolSandboxMode) -> bool {
    matches!(
        mode,
        ToolSandboxMode::WorkspaceWrite | ToolSandboxMode::IsolatedWorktree
    )
}

fn network_control(network: ProcessNetworkPolicy) -> &'static str {
    match network {
        ProcessNetworkPolicy::Deny => "denied",
        ProcessNetworkPolicy::Inherit => "inherited",
    }
}

fn inherited_controls(network: ProcessNetworkPolicy) -> SandboxControlDiagnostics {
    SandboxControlDiagnostics {
        filesystem: "inherited",
        network: network_control(network),
        process: "inherited",
        credential: "inherited",
        resource: "inherited",
        environment: "inherited",
    }
}

fn unavailable_controls() -> SandboxControlDiagnostics {
    SandboxControlDiagnostics {
        filesystem: "unavailable",
        network: "unavailable",
        process: "unavailable",
        credential: "unavailable",
        resource: "unavailable",
        environment: "unavailable",
    }
}

#[cfg(target_os = "linux")]
fn find_bubblewrap() -> Option<PathBuf> {
    [Path::new("/usr/bin/bwrap"), Path::new("/bin/bwrap")]
        .into_iter()
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|root| root.join("bwrap"))
                    .find(|path| path.is_file())
            })
        })
}

#[cfg(target_os = "macos")]
fn seatbelt_path() -> Option<PathBuf> {
    let path = Path::new("/usr/bin/sandbox-exec");
    path.is_file().then(|| path.to_path_buf())
}

#[cfg(target_os = "macos")]
fn build_macos_seatbelt_profile(
    network: ProcessNetworkPolicy,
    writable_paths: &[PathBuf],
) -> Result<String, &'static str> {
    let mut profile = String::from(
        "(version 1)\n(deny default)\n(import \"system.sb\")\n\
         (allow process*)\n(allow file-read*)\n",
    );
    if network == ProcessNetworkPolicy::Inherit {
        profile.push_str("(allow network*)\n");
    } else {
        profile.push_str("(deny network*)\n");
    }
    for path in writable_paths {
        let literal = seatbelt_path_literal(path)?;
        profile.push_str(&format!("(allow file-write* (subpath \"{literal}\"))\n"));
    }
    Ok(profile)
}

#[cfg(target_os = "macos")]
fn seatbelt_path_literal(path: &Path) -> Result<String, &'static str> {
    let value = path.to_string_lossy();
    if value
        .chars()
        .any(|value| value == '\0' || value == '\n' || value == '\r')
    {
        return Err("sandbox_path_invalid");
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
#[path = "process_sandbox_tests.rs"]
mod tests;
