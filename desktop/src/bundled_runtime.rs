use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};

const BUNDLE_SCHEMA_VERSION: u32 = 1;
const INSTALL_STATE_FILE: &str = "bundled-runtime-install-v1.json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleDescriptor {
    schema_version: u32,
    target: String,
    platform: String,
    release_tag: String,
    archive: String,
    manifest: String,
    manifest_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InstallState {
    schema_version: u32,
    manifest_sha256: String,
    executable_fingerprint: String,
    runtime_root: String,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ReconcileResult {
    NotBundled,
    Current(PathBuf),
    Installed(PathBuf),
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 180
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_descriptor(value: BundleDescriptor) -> Result<BundleDescriptor, String> {
    if value.schema_version != BUNDLE_SCHEMA_VERSION {
        return Err("embedded_runtime_schema_unsupported".into());
    }
    let expected_platform = match value.target.as_str() {
        "aarch64-apple-darwin" => "macos-aarch64",
        "x86_64-apple-darwin" => "macos-x86_64",
        _ => return Err("embedded_runtime_target_invalid".into()),
    };
    if value.platform != expected_platform
        || !value
            .release_tag
            .starts_with(&format!("{expected_platform}-"))
        || !safe_file_name(&value.release_tag)
    {
        return Err("embedded_runtime_release_tag_invalid".into());
    }
    if !safe_file_name(&value.archive)
        || !value.archive.ends_with(".tar.gz")
        || !safe_file_name(&value.manifest)
        || value.manifest != format!("{}.manifest.json", value.archive)
    {
        return Err("embedded_runtime_release_file_invalid".into());
    }
    if value.manifest_sha256.len() != 64
        || !value
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("embedded_runtime_manifest_digest_invalid".into());
    }
    Ok(value)
}

fn digest_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("embedded_runtime_manifest_read:{error}"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn executable_fingerprint() -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("embedded_runtime_executable_path:{error}"))?;
    let metadata = executable
        .metadata()
        .map_err(|error| format!("embedded_runtime_executable_metadata:{error}"))?;
    Ok(format!(
        "{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec()
    ))
}

fn runtime_root(app_data: &Path) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(configured) = std::env::var_os("APP_RUNTIME_ROOT") {
        let path = PathBuf::from(configured);
        if !path.is_absolute()
            || path.parent().is_none()
            || home.as_ref().is_some_and(|home| home == &path)
        {
            return Err("embedded_runtime_configured_root_invalid".into());
        }
        return Ok(path);
    }
    if let Some(home) = home {
        let established = home.join("agent-runtime");
        if established.join(".release-tag").is_file() {
            return Ok(established);
        }
    }
    Ok(app_data.join("runtime"))
}

fn read_install_state(path: &Path) -> Option<InstallState> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn write_install_state(path: &Path, state: &InstallState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "embedded_runtime_state_parent_missing".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("embedded_runtime_state_directory:{error}"))?;
    let temporary = parent.join(format!(".{INSTALL_STATE_FILE}.tmp"));
    let mut file = fs::File::create(&temporary)
        .map_err(|error| format!("embedded_runtime_state_create:{error}"))?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("embedded_runtime_state_encode:{error}"))?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("embedded_runtime_state_write:{error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("embedded_runtime_state_activate:{error}"))
}

fn run_logged(command: &mut Command, log_path: &Path, label: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}_start:{error}"))?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{label}_log_directory:{error}"))?;
    }
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("{label}_log_open:{error}"))?;
    log.write_all(&output.stdout)
        .and_then(|_| log.write_all(&output.stderr))
        .map_err(|error| format!("{label}_log_write:{error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.lines().rev().take(4).collect::<Vec<_>>().join(" | ");
    Err(format!("{label}_failed:{}:{detail}", output.status))
}

fn start_current_runtime(runtime: &Path, log_path: &Path) -> Result<(), String> {
    let start = runtime.join("start-all-bin.sh");
    if !start.is_file() {
        return Err("embedded_runtime_start_script_missing".into());
    }
    let mut command = Command::new("/bin/bash");
    command.arg(&start).arg("release").current_dir(runtime).env(
        "APP_PRODUCT_IDENTITY_CONFIG",
        runtime.join("configs/product_identity.toml"),
    );
    run_logged(&mut command, log_path, "embedded_runtime_start")
}

pub fn reconcile(
    app: &AppHandle,
    before_install: impl FnOnce(),
) -> Result<ReconcileResult, String> {
    let resource_root = app
        .path()
        .resource_dir()
        .map_err(|error| format!("embedded_runtime_resource_directory:{error}"))?
        .join("embedded-runtime");
    let descriptor_path = resource_root.join("bundle.json");
    if !descriptor_path.is_file() {
        return Ok(ReconcileResult::NotBundled);
    }
    let descriptor = fs::read(&descriptor_path)
        .map_err(|error| format!("embedded_runtime_descriptor_read:{error}"))?;
    let descriptor: BundleDescriptor = serde_json::from_slice(&descriptor)
        .map_err(|error| format!("embedded_runtime_descriptor_invalid:{error}"))?;
    let descriptor = validate_descriptor(descriptor)?;
    let release_dir = resource_root.join("release");
    let manifest_path = release_dir.join(&descriptor.manifest);
    if digest_file(&manifest_path)? != descriptor.manifest_sha256 {
        return Err("embedded_runtime_manifest_digest_mismatch".into());
    }
    let archive = release_dir.join(&descriptor.archive);
    if !archive.is_file() {
        return Err("embedded_runtime_archive_missing".into());
    }
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("embedded_runtime_app_data:{error}"))?;
    let runtime = runtime_root(&app_data)?;
    let fingerprint = executable_fingerprint()?;
    let state_path = app_data.join(INSTALL_STATE_FILE);
    let current_state = read_install_state(&state_path).is_some_and(|state| {
        state.schema_version == BUNDLE_SCHEMA_VERSION
            && state.manifest_sha256 == descriptor.manifest_sha256
            && state.executable_fingerprint == fingerprint
            && state.runtime_root == runtime.display().to_string()
    });
    let runtime_digest = fs::read_to_string(runtime.join(".release-manifest-digest"))
        .ok()
        .is_some_and(|value| value.trim() == descriptor.manifest_sha256);
    let log_path = app_data.join("bundled-runtime-install.log");
    if current_state && runtime_digest {
        start_current_runtime(&runtime, &log_path)?;
        return Ok(ReconcileResult::Current(runtime));
    }

    let bootstrap = resource_root.join("bootstrap");
    let deploy = bootstrap.join("deploy-github-release.sh");
    let identity = bootstrap.join("configs/product_identity.toml");
    let signers = bootstrap.join("configs/release_allowed_signers");
    let verifier = bootstrap.join("scripts/security/release_manifest.py");
    let python = bootstrap.join("toolchains/python/bin/python3");
    for required in [&deploy, &identity, &signers, &verifier, &python] {
        if !required.is_file() {
            return Err(format!(
                "embedded_runtime_bootstrap_missing:{}",
                required.display()
            ));
        }
    }
    let mut command = Command::new("/bin/bash");
    command
        .arg(&deploy)
        .arg("--root")
        .arg(&runtime)
        .arg("--platform")
        .arg(&descriptor.platform)
        .arg("--tag")
        .arg(&descriptor.release_tag)
        .arg("--local-release-dir")
        .arg(&release_dir)
        .arg("--force")
        .arg("--restart")
        .current_dir(&bootstrap)
        .env("APP_PRODUCT_IDENTITY_CONFIG", &identity)
        .env("APP_RELEASE_ALLOWED_SIGNERS_FILE", &signers)
        .env("APP_RELEASE_MANIFEST_TOOL", &verifier)
        .env("APP_PYTHON_BIN", &python);
    before_install();
    run_logged(&mut command, &log_path, "embedded_runtime_install")?;
    write_install_state(
        &state_path,
        &InstallState {
            schema_version: BUNDLE_SCHEMA_VERSION,
            manifest_sha256: descriptor.manifest_sha256,
            executable_fingerprint: fingerprint,
            runtime_root: runtime.display().to_string(),
        },
    )?;
    Ok(ReconcileResult::Installed(runtime))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> BundleDescriptor {
        BundleDescriptor {
            schema_version: 1,
            target: "aarch64-apple-darwin".into(),
            platform: "macos-aarch64".into(),
            release_tag: "macos-aarch64-1.2.3".into(),
            archive: "sample-macos-aarch64-1.2.3.tar.gz".into(),
            manifest: "sample-macos-aarch64-1.2.3.tar.gz.manifest.json".into(),
            manifest_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn descriptor_requires_matching_native_target_and_safe_files() {
        assert!(validate_descriptor(descriptor()).is_ok());
        let mut wrong = descriptor();
        wrong.platform = "macos-x86_64".into();
        assert_eq!(
            validate_descriptor(wrong).unwrap_err(),
            "embedded_runtime_release_tag_invalid"
        );
        let mut unsafe_file = descriptor();
        unsafe_file.archive = "../release.tar.gz".into();
        assert_eq!(
            validate_descriptor(unsafe_file).unwrap_err(),
            "embedded_runtime_release_file_invalid"
        );
    }
}
