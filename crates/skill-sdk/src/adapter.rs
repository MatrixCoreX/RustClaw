use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::installer::InstallControl;
use crate::manifest::{
    parse_minimum_runtime_version, BuildAdapter, BuildNetworkPolicy, LauncherKind, PackageManifest,
    PlatformArtifact,
};
use crate::process::{run_command, run_command_controlled, ProcessOutput};
use crate::receipt::{digest_file, ArtifactReceipt, LaunchProgramScope, ReceiptLaunch};
use crate::sandbox::{prepare_sandboxed_command, SandboxNetwork};
use crate::{HostPlatform, SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone)]
pub struct AdapterContext<'a> {
    pub manifest: &'a PackageManifest,
    pub workspace_root: &'a Path,
    pub manifest_dir: &'a Path,
    pub staging_root: &'a Path,
    pub cache_root: &'a Path,
    pub platform: &'a HostPlatform,
    pub target: Option<&'a str>,
    pub allow_network: bool,
    pub control: Option<&'a InstallControl>,
}

#[derive(Debug, Clone)]
pub struct PreparedPackage {
    pub adapter_version: String,
    pub artifacts: Vec<ArtifactReceipt>,
    pub launch: ReceiptLaunch,
    pub phases: Vec<String>,
}

pub fn prepare_package(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    context.manifest.validate_for_platform(context.platform)?;
    if context.manifest.build.network == BuildNetworkPolicy::ApprovalRequired
        && !context.allow_network
    {
        return Err(SkillSdkError::new(
            "build_network_approval_required",
            format!("skill={}", context.manifest.package.name),
        )
        .phase("preflight"));
    }
    fs::create_dir_all(context.staging_root.join("runtime"))?;
    fs::create_dir_all(context.cache_root)?;
    match context.manifest.build.adapter {
        BuildAdapter::Cargo => prepare_cargo(context),
        BuildAdapter::Python => prepare_python(context),
        BuildAdapter::Node => prepare_node(context),
        BuildAdapter::Go => prepare_go(context),
        BuildAdapter::Prebuilt => prepare_prebuilt(context),
        BuildAdapter::GenericProcess => prepare_generic(context),
        BuildAdapter::HttpJson => prepare_http_json(context),
    }
}

pub fn source_digest(root: &Path) -> SkillSdkResult<String> {
    let root = fs::canonicalize(root).map_err(|error| {
        SkillSdkError::new(
            "source_root_unavailable",
            format!("path={} error={error}", root.display()),
        )
        .phase("source_digest")
    })?;
    let mut files = Vec::new();
    collect_source_files(&root, &root, &mut files)?;
    files.sort();
    let mut digest = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(fs::read(&path)?);
        digest.update([0]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub fn copy_source_tree(source: &Path, destination: &Path) -> SkillSdkResult<()> {
    let source = fs::canonicalize(source)?;
    fs::create_dir_all(destination)?;
    copy_tree_inner(&source, &source, destination)
}

fn prepare_cargo(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    let cargo = find_cargo_program()?;
    let source_root = confined_workspace_path(
        context.workspace_root,
        &context.manifest.build.source_root,
        true,
    )?;
    let package = context
        .manifest
        .build
        .package
        .as_deref()
        .ok_or_else(|| missing_adapter_field("cargo", "build.package"))?;
    let binary = context
        .manifest
        .build
        .binary
        .as_deref()
        .ok_or_else(|| missing_adapter_field("cargo", "build.binary"))?;
    let cache_target = context.cache_root.join("cargo-target").join(
        context
            .target
            .unwrap_or_else(|| context.platform.target.as_deref().unwrap_or("host")),
    );
    let cargo_home = context.cache_root.join("cargo-home");
    let build_home = context.cache_root.join("home");
    fs::create_dir_all(&cache_target)?;
    fs::create_dir_all(&cargo_home)?;
    fs::create_dir_all(&build_home)?;
    if let Some(control) = context.control {
        control.phase("dependencies")?;
    }
    seed_private_cargo_home(&cargo_home, &source_root, context.manifest)?;
    let writable = vec![
        context.staging_root.to_path_buf(),
        context.cache_root.to_path_buf(),
    ];
    let mut command = sandbox_command(context, &cargo, &source_root, &writable)?;
    command
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("-p")
        .arg(package);
    if !context.allow_network {
        command.arg("--offline");
    }
    if let Some(target) = context.target {
        command.arg("--target").arg(target);
    }
    configure_clean_environment(&mut command, &build_home)?;
    configure_program_command_path(&mut command, &cargo)?;
    command.env("CARGO_HOME", &cargo_home);
    command.env("CARGO_TARGET_DIR", &cache_target);
    if let Some(rustup_home) = std::env::var_os("RUSTUP_HOME").or_else(default_rustup_home) {
        command.env("RUSTUP_HOME", rustup_home);
    }
    command.env("CARGO_BUILD_JOBS", cargo_jobs());
    require_success(
        run_adapter_command(
            context,
            &mut command,
            None,
            Duration::from_secs(7200),
            "build",
        )?,
        "build_failed",
        "cargo",
    )?;
    let built = if let Some(target) = context.target {
        cache_target.join(target).join("release").join(binary)
    } else {
        cache_target.join("release").join(binary)
    };
    let destination = context.staging_root.join(&context.manifest.run.entrypoint);
    copy_artifact(&built, &destination, true)?;
    Ok(PreparedPackage {
        adapter_version: tool_version(&cargo, &["--version"])?,
        artifacts: vec![artifact_receipt(context.staging_root, &destination, true)?],
        launch: package_launch(context, &context.manifest.run.entrypoint, Vec::new()),
        phases: vec![
            "toolchain".to_string(),
            "build".to_string(),
            "artifact".to_string(),
        ],
    })
}

fn prepare_python(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    require_native_target(context, "python")?;
    let python = find_program_with_override("python3", "APP_PYTHON_BIN")?;
    validate_python_requirement(
        &python,
        context
            .manifest
            .build
            .options
            .get("python")
            .map(String::as_str),
    )?;
    let source_root = source_root(context)?;
    let runtime_source = context.staging_root.join("runtime/src");
    copy_source_tree(&source_root, &runtime_source)?;
    let venv = context.staging_root.join("runtime/venv");
    let build_home = context.cache_root.join("home");
    let pip_cache = context.cache_root.join("pip");
    fs::create_dir_all(&build_home)?;
    fs::create_dir_all(&pip_cache)?;
    let writable = vec![
        context.staging_root.to_path_buf(),
        context.cache_root.to_path_buf(),
    ];
    let mut create = sandbox_command(context, &python, context.workspace_root, &writable)?;
    create.arg("-m").arg("venv").arg(&venv);
    configure_clean_environment(&mut create, &build_home)?;
    require_success(
        run_adapter_command(
            context,
            &mut create,
            None,
            Duration::from_secs(300),
            "prepare_environment",
        )?,
        "build_environment_failed",
        "python",
    )?;
    remove_redundant_python_venv_aliases(&venv)?;
    let venv_python = venv_python_path(&venv);
    let lockfile = confined_source_path(
        &source_root,
        context
            .manifest
            .build
            .lockfile
            .as_deref()
            .ok_or_else(|| missing_adapter_field("python", "build.lockfile"))?,
        true,
    )?;
    let mut install = sandbox_command(context, &venv_python, context.workspace_root, &writable)?;
    install
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("--require-hashes")
        .arg("--no-compile")
        .arg("--disable-pip-version-check")
        .arg("-r")
        .arg(lockfile);
    if !context.allow_network {
        install.arg("--no-index");
    }
    configure_clean_environment(&mut install, &build_home)?;
    install.env("PIP_CACHE_DIR", &pip_cache);
    install.env("PYTHONNOUSERSITE", "1");
    require_success(
        run_adapter_command(
            context,
            &mut install,
            None,
            Duration::from_secs(1800),
            "dependencies",
        )?,
        "dependency_install_failed",
        "python",
    )?;
    remove_python_bytecode_caches(context.staging_root.join("runtime").as_path())?;
    rewrite_python_console_scripts_for_relocation(&venv)?;
    materialize_file_symlink(&venv_python)?;
    let relative_python = relative_string(context.staging_root, &venv_python)?;
    let entrypoint = context.staging_root.join(&context.manifest.run.entrypoint);
    if !entrypoint.is_file() {
        return Err(SkillSdkError::new(
            "build_entrypoint_missing",
            entrypoint.display().to_string(),
        )
        .phase("artifact"));
    }
    let mut args = vec![context.manifest.run.entrypoint.clone()];
    args.extend(context.manifest.run.args.clone());
    let mut environment = BTreeMap::new();
    environment.insert("PYTHONNOUSERSITE".to_string(), "1".to_string());
    environment.insert("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string());
    Ok(PreparedPackage {
        adapter_version: tool_version(&python, &["--version"])?,
        artifacts: collect_runtime_artifacts(context.staging_root)?,
        launch: ReceiptLaunch {
            launcher: LauncherKind::Python,
            program: relative_python,
            program_scope: LaunchProgramScope::Package,
            args,
            working_directory: context.manifest.run.working_directory.clone(),
            environment,
            environment_allowlist: context.manifest.run.environment_allowlist.clone(),
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: None,
        },
        phases: vec![
            "toolchain".to_string(),
            "prepare_environment".to_string(),
            "dependencies".to_string(),
            "artifact".to_string(),
        ],
    })
}

fn validate_python_requirement(program: &Path, requirement: Option<&str>) -> SkillSdkResult<()> {
    let Some(requirement) = requirement else {
        return Ok(());
    };
    let minimum = parse_minimum_runtime_version(requirement, "build.options.python")?;
    let actual_version = tool_version(program, &["--version"])?;
    let actual_text = actual_version
        .strip_prefix("Python ")
        .unwrap_or(actual_version.as_str());
    let actual =
        parse_minimum_runtime_version(&format!(">={actual_text}"), "python.detected_version")
            .map_err(|error| {
                SkillSdkError::new(
                    "toolchain_version_failed",
                    format!("program={} detail={}", program.display(), error.detail),
                )
                .phase("preflight")
            })?;
    if actual < minimum {
        return Err(SkillSdkError::new(
            "toolchain_version_unsupported",
            format!(
                "program={} required={} actual={actual_text}",
                program.display(),
                requirement
            ),
        )
        .phase("preflight"));
    }
    Ok(())
}

fn prepare_node(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    require_native_target(context, "node")?;
    let node = find_program("node")?;
    let npm = find_program("npm")?;
    let source_root = source_root(context)?;
    let runtime_source = context.staging_root.join("runtime/src");
    copy_source_tree(&source_root, &runtime_source)?;
    if !runtime_source.join("package.json").is_file() {
        return Err(missing_adapter_field("node", "package.json"));
    }
    let lockfile = context
        .manifest
        .build
        .lockfile
        .as_deref()
        .ok_or_else(|| missing_adapter_field("node", "build.lockfile"))?;
    if !runtime_source.join(lockfile).is_file() {
        return Err(missing_adapter_field("node", lockfile));
    }
    let npm_cache = context.cache_root.join("npm");
    let build_home = context.cache_root.join("home");
    fs::create_dir_all(&npm_cache)?;
    fs::create_dir_all(&build_home)?;
    let writable = vec![
        context.staging_root.to_path_buf(),
        context.cache_root.to_path_buf(),
    ];
    let mut install = sandbox_command(context, &npm, &runtime_source, &writable)?;
    install.arg("ci");
    if !context.manifest.build.lifecycle_scripts {
        install.arg("--ignore-scripts");
    }
    if !context.allow_network {
        install.arg("--offline");
    }
    configure_clean_environment(&mut install, &build_home)?;
    install.env("npm_config_cache", &npm_cache);
    require_success(
        run_adapter_command(
            context,
            &mut install,
            None,
            Duration::from_secs(1800),
            "dependencies",
        )?,
        "dependency_install_failed",
        "node",
    )?;
    let entrypoint = context.staging_root.join(&context.manifest.run.entrypoint);
    if !entrypoint.is_file() {
        return Err(SkillSdkError::new(
            "build_entrypoint_missing",
            entrypoint.display().to_string(),
        )
        .phase("artifact"));
    }
    let mut args = vec![context.manifest.run.entrypoint.clone()];
    args.extend(context.manifest.run.args.clone());
    Ok(PreparedPackage {
        adapter_version: tool_version(&node, &["--version"])?,
        artifacts: collect_runtime_artifacts(context.staging_root)?,
        launch: trusted_runtime_launch(context, &node, LauncherKind::Node, args)?,
        phases: vec![
            "toolchain".to_string(),
            "dependencies".to_string(),
            "artifact".to_string(),
        ],
    })
}

fn prepare_go(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    let go = find_program("go")?;
    let source_root = source_root(context)?;
    let lockfile = context
        .manifest
        .build
        .lockfile
        .as_deref()
        .ok_or_else(|| missing_adapter_field("go", "build.lockfile"))?;
    confined_source_path(&source_root, lockfile, true)?;
    confined_source_path(&source_root, "go.mod", true)?;
    let destination = context.staging_root.join(&context.manifest.run.entrypoint);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let go_cache = context.cache_root.join("go-build");
    let go_modules = context.cache_root.join("go-mod");
    let build_home = context.cache_root.join("home");
    fs::create_dir_all(&go_cache)?;
    fs::create_dir_all(&go_modules)?;
    fs::create_dir_all(&build_home)?;
    let writable = vec![
        context.staging_root.to_path_buf(),
        context.cache_root.to_path_buf(),
    ];
    let mut build = sandbox_command(context, &go, &source_root, &writable)?;
    build
        .arg("build")
        .arg("-mod=readonly")
        .arg("-o")
        .arg(&destination)
        .arg(
            context
                .manifest
                .build
                .options
                .get("main")
                .map(String::as_str)
                .unwrap_or("."),
        );
    configure_clean_environment(&mut build, &build_home)?;
    build.env("GOCACHE", &go_cache);
    build.env("GOMODCACHE", &go_modules);
    if context.target.is_some() {
        build.env(
            "GOOS",
            match context.platform.os.as_str() {
                "macos" => "darwin",
                value => value,
            },
        );
        build.env(
            "GOARCH",
            match context.platform.arch.as_str() {
                "x86_64" => "amd64",
                "aarch64" => "arm64",
                "armv7" => "arm",
                value => value,
            },
        );
        build.env("CGO_ENABLED", "0");
        if context.platform.arch == "armv7" {
            build.env("GOARM", "7");
        }
    }
    if !context.allow_network {
        build.env("GOPROXY", "off");
    }
    require_success(
        run_adapter_command(
            context,
            &mut build,
            None,
            Duration::from_secs(3600),
            "build",
        )?,
        "build_failed",
        "go",
    )?;
    set_executable(&destination)?;
    Ok(PreparedPackage {
        adapter_version: tool_version(&go, &["version"])?,
        artifacts: vec![artifact_receipt(context.staging_root, &destination, true)?],
        launch: package_launch(context, &context.manifest.run.entrypoint, Vec::new()),
        phases: vec![
            "toolchain".to_string(),
            "build".to_string(),
            "artifact".to_string(),
        ],
    })
}

fn prepare_prebuilt(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    if let Some(control) = context.control {
        control.phase("dependencies")?;
    }
    let artifact = select_prebuilt_artifact(&context.manifest.build.artifacts, context.platform)?;
    let source = crate::prebuilt::resolve_source(
        artifact,
        context.manifest_dir,
        context.cache_root,
        context.allow_network,
    )?;
    if let Some(control) = context.control {
        control.phase("build")?;
    }
    crate::prebuilt::install(
        &source,
        artifact,
        context.staging_root,
        &context.manifest.run.entrypoint,
    )?;
    Ok(PreparedPackage {
        adapter_version: "prebuilt-v1".to_string(),
        artifacts: collect_runtime_artifacts(context.staging_root)?,
        launch: package_launch(context, &context.manifest.run.entrypoint, Vec::new()),
        phases: vec!["select_platform".to_string(), "artifact".to_string()],
    })
}

fn prepare_generic(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    require_native_target(context, "generic_process")?;
    let source_root = source_root(context)?;
    let runtime_source = context.staging_root.join("runtime/src");
    copy_source_tree(&source_root, &runtime_source)?;
    let entrypoint = context.staging_root.join(&context.manifest.run.entrypoint);
    if !entrypoint.is_file() {
        return Err(SkillSdkError::new(
            "build_entrypoint_missing",
            entrypoint.display().to_string(),
        )
        .phase("artifact"));
    }
    let launch = match context.manifest.run.launcher {
        LauncherKind::Native | LauncherKind::Process => {
            set_executable(&entrypoint)?;
            package_launch(context, &context.manifest.run.entrypoint, Vec::new())
        }
        LauncherKind::Java => trusted_runtime_launch(
            context,
            &find_program("java")?,
            LauncherKind::Java,
            std::iter::once(context.manifest.run.entrypoint.clone())
                .chain(context.manifest.run.args.clone())
                .collect(),
        )?,
        LauncherKind::Dotnet => trusted_runtime_launch(
            context,
            &find_program("dotnet")?,
            LauncherKind::Dotnet,
            std::iter::once(context.manifest.run.entrypoint.clone())
                .chain(context.manifest.run.args.clone())
                .collect(),
        )?,
        _ => {
            return Err(SkillSdkError::new(
                "generic_launcher_unsupported",
                format!("launcher={:?}", context.manifest.run.launcher),
            ))
        }
    };
    Ok(PreparedPackage {
        adapter_version: "generic-process-v1".to_string(),
        artifacts: collect_runtime_artifacts(context.staging_root)?,
        launch,
        phases: vec!["artifact".to_string()],
    })
}

fn prepare_http_json(context: &AdapterContext<'_>) -> SkillSdkResult<PreparedPackage> {
    let endpoint = context
        .manifest
        .build
        .options
        .get("endpoint")
        .filter(|value| value.starts_with("https://"))
        .ok_or_else(|| missing_adapter_field("http_json", "build.options.endpoint"))?;
    let marker = context.staging_root.join("runtime/http-json-adapter");
    fs::write(
        &marker,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "adapter": "http_json",
            "endpoint": endpoint,
        }))?,
    )?;
    Ok(PreparedPackage {
        adapter_version: "http-json-v1".to_string(),
        artifacts: vec![artifact_receipt(context.staging_root, &marker, false)?],
        launch: ReceiptLaunch {
            launcher: LauncherKind::HttpJson,
            program: "runtime/http-json-adapter".to_string(),
            program_scope: LaunchProgramScope::Package,
            args: Vec::new(),
            working_directory: ".".to_string(),
            environment: BTreeMap::new(),
            environment_allowlist: context.manifest.run.environment_allowlist.clone(),
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: Some(endpoint.clone()),
        },
        phases: vec!["endpoint_validation".to_string()],
    })
}

fn source_root(context: &AdapterContext<'_>) -> SkillSdkResult<PathBuf> {
    confined_workspace_path(
        context.workspace_root,
        &context.manifest.build.source_root,
        true,
    )
}

fn require_native_target(context: &AdapterContext<'_>, adapter: &str) -> SkillSdkResult<()> {
    let host = HostPlatform::current();
    if context.target.is_some()
        && (context.platform.os != host.os || context.platform.arch != host.arch)
    {
        return Err(SkillSdkError::new(
            "adapter_cross_target_unsupported",
            format!(
                "adapter={adapter} host={}/{} target={}/{}",
                host.os, host.arch, context.platform.os, context.platform.arch
            ),
        )
        .phase("preflight"));
    }
    Ok(())
}

fn sandbox_command(
    context: &AdapterContext<'_>,
    program: &Path,
    execution_root: &Path,
    writable: &[PathBuf],
) -> SkillSdkResult<Command> {
    let network = if context.allow_network {
        SandboxNetwork::Allow
    } else {
        SandboxNetwork::Deny
    };
    Ok(prepare_sandboxed_command(program, execution_root, writable, network)?.command)
}

fn package_launch(
    context: &AdapterContext<'_>,
    program: &str,
    prefix_args: Vec<String>,
) -> ReceiptLaunch {
    let mut args = prefix_args;
    args.extend(context.manifest.run.args.clone());
    ReceiptLaunch {
        launcher: context.manifest.run.launcher,
        program: program.to_string(),
        program_scope: LaunchProgramScope::Package,
        args,
        working_directory: context.manifest.run.working_directory.clone(),
        environment: BTreeMap::new(),
        environment_allowlist: context.manifest.run.environment_allowlist.clone(),
        trusted_runtime_sha256: None,
        trusted_runtime_version: None,
        remote_endpoint: None,
    }
}

fn trusted_runtime_launch(
    context: &AdapterContext<'_>,
    runtime: &Path,
    launcher: LauncherKind,
    args: Vec<String>,
) -> SkillSdkResult<ReceiptLaunch> {
    Ok(ReceiptLaunch {
        launcher,
        program: runtime.to_string_lossy().into_owned(),
        program_scope: LaunchProgramScope::TrustedRuntime,
        args,
        working_directory: context.manifest.run.working_directory.clone(),
        environment: BTreeMap::new(),
        environment_allowlist: context.manifest.run.environment_allowlist.clone(),
        trusted_runtime_sha256: Some(digest_file(runtime)?),
        trusted_runtime_version: Some(tool_version(runtime, &["--version"])?),
        remote_endpoint: None,
    })
}

fn collect_runtime_artifacts(root: &Path) -> SkillSdkResult<Vec<ArtifactReceipt>> {
    let runtime = root.join("runtime");
    materialize_runtime_tree_symlinks(&runtime)?;
    let mut files = Vec::new();
    collect_runtime_files(&runtime, &runtime, &mut files)?;
    files.sort();
    files
        .into_iter()
        .map(|relative| {
            let path = runtime.join(&relative);
            artifact_receipt(root, &path, is_executable(&path))
        })
        .collect()
}

fn collect_runtime_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<PathBuf>,
) -> SkillSdkResult<()> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(SkillSdkError::new(
                "runtime_symlink_unmaterialized",
                path.display().to_string(),
            )
            .phase("artifact"));
        }
        if metadata.is_dir() {
            collect_runtime_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

fn materialize_runtime_tree_symlinks(root: &Path) -> SkillSdkResult<()> {
    let canonical_root = fs::canonicalize(root)?;
    let mut links = Vec::new();
    collect_runtime_symlinks(root, &mut links)?;
    links.sort_by_key(|path| path.components().count());
    let mut budget = RuntimeCopyBudget::default();
    for link in links {
        if !fs::symlink_metadata(&link)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        let target = fs::canonicalize(&link).map_err(|error| {
            SkillSdkError::new(
                "runtime_symlink_broken",
                format!("path={} error={error}", link.display()),
            )
            .phase("artifact")
        })?;
        if target.is_file() {
            materialize_file_symlink(&link)?;
        } else if target.is_dir() && target.starts_with(&canonical_root) {
            fs::remove_file(&link)?;
            copy_runtime_directory(&target, &link, &canonical_root, &mut budget)?;
        } else {
            return Err(SkillSdkError::new(
                "runtime_symlink_target_invalid",
                format!("path={} target={}", link.display(), target.display()),
            )
            .phase("artifact"));
        }
    }
    Ok(())
}

fn collect_runtime_symlinks(current: &Path, links: &mut Vec<PathBuf>) -> SkillSdkResult<()> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            links.push(path);
        } else if metadata.is_dir() {
            collect_runtime_symlinks(&path, links)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct RuntimeCopyBudget {
    files: usize,
    bytes: u64,
}

fn copy_runtime_directory(
    source: &Path,
    destination: &Path,
    canonical_root: &Path,
    budget: &mut RuntimeCopyBudget,
) -> SkillSdkResult<()> {
    fs::create_dir(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            let target = fs::canonicalize(&source_path)?;
            if target.is_file() {
                copy_runtime_file(&target, &destination_path, budget)?;
            } else if target.is_dir() && target.starts_with(canonical_root) {
                return Err(SkillSdkError::new(
                    "runtime_nested_directory_symlink_forbidden",
                    format!("path={} target={}", source_path.display(), target.display()),
                )
                .phase("artifact"));
            } else {
                return Err(SkillSdkError::new(
                    "runtime_symlink_target_invalid",
                    format!("path={} target={}", source_path.display(), target.display()),
                )
                .phase("artifact"));
            }
        } else if metadata.is_dir() {
            copy_runtime_directory(&source_path, &destination_path, canonical_root, budget)?;
        } else if metadata.is_file() {
            copy_runtime_file(&source_path, &destination_path, budget)?;
        }
    }
    Ok(())
}

fn copy_runtime_file(
    source: &Path,
    destination: &Path,
    budget: &mut RuntimeCopyBudget,
) -> SkillSdkResult<()> {
    budget.files = budget.files.saturating_add(1);
    budget.bytes = budget.bytes.saturating_add(fs::metadata(source)?.len());
    if budget.files > 100_000 || budget.bytes > 1024 * 1024 * 1024 {
        return Err(SkillSdkError::new(
            "runtime_symlink_materialization_limit",
            format!("files={} bytes={}", budget.files, budget.bytes),
        )
        .phase("artifact"));
    }
    fs::copy(source, destination)?;
    Ok(())
}

fn materialize_file_symlink(path: &Path) -> SkillSdkResult<()> {
    if !fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(());
    }
    let target = fs::canonicalize(path)?;
    if !target.is_file() {
        return Err(SkillSdkError::new(
            "runtime_symlink_target_invalid",
            format!("path={} target={}", path.display(), target.display()),
        )
        .phase("artifact"));
    }
    let replacement = path.with_extension("agent-materialized");
    fs::copy(&target, &replacement)?;
    set_executable(&replacement)?;
    fs::remove_file(path)?;
    fs::rename(replacement, path)?;
    Ok(())
}

fn artifact_receipt(
    install_root: &Path,
    path: &Path,
    executable: bool,
) -> SkillSdkResult<ArtifactReceipt> {
    Ok(ArtifactReceipt {
        path: relative_string(install_root, path)?,
        sha256: digest_file(path)?,
        size_bytes: fs::metadata(path)?.len(),
        executable,
    })
}

fn copy_artifact(source: &Path, destination: &Path, executable: bool) -> SkillSdkResult<()> {
    if !source.is_file() {
        return Err(
            SkillSdkError::new("build_artifact_missing", source.display().to_string())
                .phase("artifact"),
        );
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    if executable {
        set_executable(destination)?;
    }
    Ok(())
}

fn select_prebuilt_artifact<'a>(
    artifacts: &'a [PlatformArtifact],
    platform: &HostPlatform,
) -> SkillSdkResult<&'a PlatformArtifact> {
    artifacts
        .iter()
        .find(|artifact| {
            crate::platform::normalize_os(&artifact.os) == Some(platform.os.as_str())
                && crate::platform::normalize_arch(&artifact.arch) == Some(platform.arch.as_str())
        })
        .ok_or_else(|| {
            SkillSdkError::new(
                "prebuilt_platform_artifact_missing",
                format!("os={} arch={}", platform.os, platform.arch),
            )
            .phase("preflight")
        })
}

fn require_success(output: ProcessOutput, code: &str, adapter: &str) -> SkillSdkResult<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(SkillSdkError::new(
        code,
        format!(
            "adapter={adapter} exit_code={:?} stdout={} stderr={}",
            output.status.code(),
            bounded_text(&crate::secret_scan::redact_diagnostics(&stdout)),
            bounded_text(&crate::secret_scan::redact_diagnostics(&stderr))
        ),
    )
    .phase("build"))
}

fn run_adapter_command(
    context: &AdapterContext<'_>,
    command: &mut Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
    phase: &str,
) -> SkillSdkResult<ProcessOutput> {
    if let Some(control) = context.control {
        control.phase(phase)?;
    }
    run_command_controlled(
        command,
        stdin,
        timeout,
        phase,
        context.control.map(InstallControl::cancelled_flag),
    )
}

fn bounded_text(value: &str) -> String {
    value.chars().take(4000).collect()
}

fn find_program(name: &str) -> SkillSdkResult<PathBuf> {
    find_program_in_environment(name, std::env::var_os("PATH").as_deref())
}

fn find_cargo_program() -> SkillSdkResult<PathBuf> {
    find_cargo_program_in_environment(
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("CARGO_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

fn find_cargo_program_in_environment(
    path: Option<&OsStr>,
    cargo_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> SkillSdkResult<PathBuf> {
    let mut directories = path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(cargo_home) = cargo_home.filter(|value| !value.is_empty()) {
        push_unique_path(&mut directories, PathBuf::from(cargo_home).join("bin"));
    }
    if let Some(home) = home.filter(|value| !value.is_empty()) {
        push_unique_path(
            &mut directories,
            PathBuf::from(home).join(".cargo").join("bin"),
        );
    }
    find_program_in_directories("cargo", directories)
}

fn find_program_in_environment(name: &str, path: Option<&OsStr>) -> SkillSdkResult<PathBuf> {
    let directories = path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    find_program_in_directories(name, directories)
}

fn find_program_in_directories(
    name: &str,
    directories: impl IntoIterator<Item = PathBuf>,
) -> SkillSdkResult<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        return Err(SkillSdkError::new(
            "toolchain_name_invalid",
            format!("name={name}"),
        ));
    }
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|path| path.is_file() && is_executable(path))
        .ok_or_else(|| {
            SkillSdkError::new("toolchain_missing", format!("program={name}")).phase("preflight")
        })
}

fn push_unique_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|path| path == &candidate) {
        paths.push(candidate);
    }
}

fn command_path_for_program(program: &Path) -> SkillSdkResult<OsString> {
    let mut directories = Vec::new();
    if let Some(parent) = program
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        push_unique_path(&mut directories, parent.to_path_buf());
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            push_unique_path(&mut directories, directory);
        }
    }
    std::env::join_paths(directories).map_err(|error| {
        SkillSdkError::new(
            "toolchain_path_invalid",
            format!("program={} error={error}", program.display()),
        )
        .phase("preflight")
    })
}

fn configure_program_command_path(command: &mut Command, program: &Path) -> SkillSdkResult<()> {
    command.env("PATH", command_path_for_program(program)?);
    Ok(())
}

fn find_program_with_override(name: &str, variable: &str) -> SkillSdkResult<PathBuf> {
    match std::env::var_os(variable).filter(|value| !value.is_empty()) {
        Some(value) => validate_program_override(variable, Path::new(&value)),
        None => find_program(name),
    }
}

fn validate_program_override(variable: &str, path: &Path) -> SkillSdkResult<PathBuf> {
    if !path.is_absolute() {
        return Err(SkillSdkError::new(
            "toolchain_override_invalid",
            format!("variable={variable} reason=path_not_absolute"),
        )
        .phase("preflight"));
    }
    let resolved = fs::canonicalize(path).map_err(|error| {
        SkillSdkError::new(
            "toolchain_override_invalid",
            format!("variable={variable} error={error}"),
        )
        .phase("preflight")
    })?;
    if !resolved.is_file() || !is_executable(&resolved) {
        return Err(SkillSdkError::new(
            "toolchain_override_invalid",
            format!("variable={variable} reason=not_executable"),
        )
        .phase("preflight"));
    }
    Ok(resolved)
}

fn tool_version(program: &Path, args: &[&str]) -> SkillSdkResult<String> {
    let mut command = Command::new(program);
    command.args(args).env_clear();
    configure_program_command_path(&mut command, program)?;
    if let Some(home) = std::env::var_os("RUSTUP_HOME").or_else(default_rustup_home) {
        command.env("RUSTUP_HOME", home);
    }
    let output = run_command(&mut command, None, Duration::from_secs(15), "preflight")?;
    if !output.status.success() {
        return Err(SkillSdkError::new(
            "toolchain_version_failed",
            format!("program={}", program.display()),
        )
        .phase("preflight"));
    }
    let value = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    Ok(String::from_utf8_lossy(&value).trim().to_string())
}

fn configure_clean_environment(command: &mut Command, home: &Path) -> SkillSdkResult<()> {
    let temp = home
        .parent()
        .ok_or_else(|| SkillSdkError::new("build_home_parent_missing", home.display().to_string()))?
        .join("tmp");
    fs::create_dir_all(&temp)?;
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.env("HOME", home);
    command.env("TMPDIR", &temp);
    command.env("TMP", &temp);
    command.env("TEMP", &temp);
    command.env("LANG", "C.UTF-8");
    command.env("LC_ALL", "C.UTF-8");
    Ok(())
}

fn default_rustup_home() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup").into_os_string())
}

fn cargo_jobs() -> String {
    let detected = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    cargo_jobs_for(std::env::var("CARGO_BUILD_JOBS").ok().as_deref(), detected)
}

fn cargo_jobs_for(configured: Option<&str>, detected: usize) -> String {
    configured
        .map(str::trim)
        .filter(|value| value.parse::<usize>().is_ok_and(|jobs| jobs > 0))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| detected.max(1).to_string())
}

fn seed_private_cargo_home(
    private_home: &Path,
    source_root: &Path,
    manifest: &PackageManifest,
) -> SkillSdkResult<()> {
    let lockfile = manifest
        .build
        .lockfile
        .as_deref()
        .ok_or_else(|| missing_adapter_field("cargo", "build.lockfile"))?;
    let lock_path = confined_source_path(source_root, lockfile, true)?;
    let parsed = fs::read_to_string(&lock_path)?
        .parse::<toml::Value>()
        .map_err(|error| {
            SkillSdkError::new(
                "cargo_lockfile_invalid",
                format!("path={} error={error}", lock_path.display()),
            )
            .phase("dependencies")
        })?;
    let packages = parsed
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|package| {
            let source = package.get("source")?.as_str()?;
            if !source.starts_with("registry+") {
                return None;
            }
            Some((
                package.get("name")?.as_str()?.to_string(),
                package.get("version")?.as_str()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    if packages.is_empty() {
        return Ok(());
    }
    let source_home = std::env::var_os("APP_CARGO_SOURCE_CACHE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .ok_or_else(|| {
            SkillSdkError::new("cargo_source_cache_unavailable", "cargo home is unknown")
                .phase("dependencies")
        })?;
    seed_locked_cargo_packages(private_home, &source_home, &packages)
}

fn seed_locked_cargo_packages(
    private_home: &Path,
    source_home: &Path,
    packages: &[(String, String)],
) -> SkillSdkResult<()> {
    let source_indexes = sorted_directories(&source_home.join("registry/index"))?;
    let source_caches = sorted_directories(&source_home.join("registry/cache"))?;
    for (name, version) in packages {
        let index_relative = crates_io_index_relative(&name);
        for source_index in &source_indexes {
            let source_entry = source_index.join(".cache").join(&index_relative);
            if !source_entry.is_file() {
                continue;
            }
            let index_name = source_index.file_name().ok_or_else(|| {
                SkillSdkError::new(
                    "cargo_source_cache_invalid",
                    source_index.display().to_string(),
                )
            })?;
            let destination_index = private_home.join("registry/index").join(index_name);
            copy_if_missing(
                &source_index.join("config.json"),
                &destination_index.join("config.json"),
            )?;
            copy_if_missing(
                &source_entry,
                &destination_index.join(".cache").join(&index_relative),
            )?;
        }
        let crate_name = format!("{name}-{version}.crate");
        for source_cache in &source_caches {
            let source_crate = source_cache.join(&crate_name);
            if !source_crate.is_file() {
                continue;
            }
            let cache_name = source_cache.file_name().ok_or_else(|| {
                SkillSdkError::new(
                    "cargo_source_cache_invalid",
                    source_cache.display().to_string(),
                )
            })?;
            copy_if_missing(
                &source_crate,
                &private_home
                    .join("registry/cache")
                    .join(cache_name)
                    .join(&crate_name),
            )?;
        }
    }
    Ok(())
}

fn sorted_directories(root: &Path) -> SkillSdkResult<Vec<PathBuf>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut directories = fs::read_dir(root)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort();
    Ok(directories)
}

fn crates_io_index_relative(name: &str) -> PathBuf {
    let normalized = name.to_ascii_lowercase();
    match normalized.len() {
        1 => PathBuf::from("1").join(normalized),
        2 => PathBuf::from("2").join(normalized),
        3 => PathBuf::from("3").join(&normalized[..1]).join(normalized),
        _ => PathBuf::from(&normalized[..2])
            .join(&normalized[2..4])
            .join(normalized),
    }
}

fn copy_if_missing(source: &Path, destination: &Path) -> SkillSdkResult<()> {
    if destination.is_file() {
        return Ok(());
    }
    if !source.is_file() {
        return Err(SkillSdkError::new(
            "cargo_source_cache_file_missing",
            source.display().to_string(),
        )
        .phase("dependencies"));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

fn confined_workspace_path(
    root: &Path,
    relative: &str,
    directory: bool,
) -> SkillSdkResult<PathBuf> {
    let root = fs::canonicalize(root)?;
    let path = fs::canonicalize(root.join(relative))?;
    if !path.starts_with(&root) || (directory && !path.is_dir()) || (!directory && !path.is_file())
    {
        return Err(
            SkillSdkError::new("source_path_escape", format!("path={}", path.display()))
                .phase("preflight"),
        );
    }
    Ok(path)
}

fn confined_source_path(root: &Path, relative: &str, file: bool) -> SkillSdkResult<PathBuf> {
    let root = fs::canonicalize(root)?;
    let path = fs::canonicalize(root.join(relative)).map_err(|error| {
        SkillSdkError::new(
            "source_path_missing",
            format!("path={relative} error={error}"),
        )
    })?;
    if !path.starts_with(&root) || (file && !path.is_file()) || (!file && !path.is_dir()) {
        return Err(SkillSdkError::new(
            "source_path_escape",
            format!("path={}", path.display()),
        ));
    }
    Ok(path)
}

fn relative_string(root: &Path, path: &Path) -> SkillSdkResult<String> {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|_| {
            SkillSdkError::new(
                "artifact_path_escape",
                format!("root={} path={}", root.display(), path.display()),
            )
        })
}

fn collect_source_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<PathBuf>,
) -> SkillSdkResult<()> {
    if !current.is_dir() {
        return Err(SkillSdkError::new(
            "source_directory_missing",
            current.display().to_string(),
        ));
    }
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let name = entry.file_name();
        if ignored_source_name(&name) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            return Err(
                SkillSdkError::new("source_symlink_forbidden", path.display().to_string())
                    .phase("source_digest"),
            );
        }
        if metadata.is_dir() {
            collect_source_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

fn copy_tree_inner(root: &Path, current: &Path, destination: &Path) -> SkillSdkResult<()> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let metadata = fs::symlink_metadata(&source)?;
        if ignored_source_name(&entry.file_name()) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            return Err(SkillSdkError::new(
                "source_symlink_forbidden",
                source.display().to_string(),
            )
            .phase("copy_source"));
        }
        let relative = source
            .strip_prefix(root)
            .map_err(|_| SkillSdkError::new("source_path_escape", source.display().to_string()))?;
        let target = destination.join(relative);
        if metadata.is_dir() {
            fs::create_dir_all(&target)?;
            copy_tree_inner(root, &source, destination)?;
        } else if metadata.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

fn ignored_source_name(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git" | "target" | "node_modules" | "__pycache__" | ".venv")
    )
}

fn missing_adapter_field(adapter: &str, field: &str) -> SkillSdkError {
    SkillSdkError::new(
        "adapter_field_missing",
        format!("adapter={adapter} field={field}"),
    )
    .phase("preflight")
}

#[cfg(unix)]
fn set_executable(path: &Path) -> SkillSdkResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> SkillSdkResult<()> {
    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn venv_python_path(venv: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        venv.join("Scripts/python.exe")
    }
    #[cfg(not(windows))]
    {
        venv.join("bin/python")
    }
}

fn remove_redundant_python_venv_aliases(venv: &Path) -> SkillSdkResult<()> {
    #[cfg(not(windows))]
    {
        // Python creates `lib64 -> lib` on many 64-bit Unix hosts even though
        // the interpreter's venv sys.path uses `lib`. Keeping that alias would
        // make the receipt hardening pass duplicate every dependency before it
        // can remove symlinks, which is especially wasteful for ML runtimes.
        let alias = venv.join("lib64");
        if fs::symlink_metadata(&alias)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            let target = fs::canonicalize(&alias)?;
            let canonical_lib = fs::canonicalize(venv.join("lib"))?;
            if target == canonical_lib {
                fs::remove_file(alias)?;
            }
        }
    }
    Ok(())
}

fn remove_python_bytecode_caches(root: &Path) -> SkillSdkResult<()> {
    let mut entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            if entry.file_name() == "__pycache__" {
                fs::remove_dir_all(path)?;
            } else {
                remove_python_bytecode_caches(&path)?;
            }
        } else if metadata.is_file()
            && path
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| matches!(extension, "pyc" | "pyo"))
        {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn rewrite_python_console_scripts_for_relocation(venv: &Path) -> SkillSdkResult<usize> {
    let bin = venv.join("bin");
    let bin_text = bin.to_string_lossy();
    let shebang_prefix = format!("#!{bin_text}");
    let portable_header =
        b"#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python\" \"$0\" \"$@\"\n' '''\n";
    let mut rewritten_count = 0;

    for entry in fs::read_dir(&bin)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let bytes = fs::read(&path)?;
        let Some(line_end) = bytes.iter().position(|byte| *byte == b'\n') else {
            continue;
        };
        let Ok(first_line) = std::str::from_utf8(&bytes[..line_end]) else {
            continue;
        };
        let body_start = if first_line.starts_with(&shebang_prefix) {
            Some(line_end + 1)
        } else if first_line == "#!/bin/sh" {
            let remaining = &bytes[line_end + 1..];
            let Some(second_line_end) = remaining.iter().position(|byte| *byte == b'\n') else {
                continue;
            };
            let second_line = &remaining[..second_line_end];
            let third_and_body = &remaining[second_line_end + 1..];
            let Some(third_line_end) = third_and_body.iter().position(|byte| *byte == b'\n') else {
                continue;
            };
            let Ok(second_line) = std::str::from_utf8(second_line) else {
                continue;
            };
            let Ok(third_line) = std::str::from_utf8(&third_and_body[..third_line_end]) else {
                continue;
            };
            (second_line.starts_with("'''exec' ")
                && second_line.contains(bin_text.as_ref())
                && third_line == "' '''")
                .then_some(line_end + second_line_end + third_line_end + 3)
        } else {
            None
        };
        let Some(body_start) = body_start else {
            continue;
        };

        let mut relocated = Vec::with_capacity(portable_header.len() + bytes.len() - body_start);
        relocated.extend_from_slice(portable_header);
        relocated.extend_from_slice(&bytes[body_start..]);
        fs::write(&path, relocated)?;
        rewritten_count += 1;
    }

    Ok(rewritten_count)
}

#[cfg(not(unix))]
fn rewrite_python_console_scripts_for_relocation(_venv: &Path) -> SkillSdkResult<usize> {
    Ok(0)
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
