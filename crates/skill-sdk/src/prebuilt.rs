use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::manifest::{ArchiveFormat, PlatformArtifact};
use crate::receipt::digest_file;
use crate::{SkillSdkError, SkillSdkResult};

const MAX_PREBUILT_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_ARCHIVE_EXPANDED_BYTES: u64 = 1024 * 1024 * 1024;

pub(crate) fn resolve_source(
    artifact: &PlatformArtifact,
    manifest_dir: &Path,
    cache_root: &Path,
    allow_network: bool,
) -> SkillSdkResult<PathBuf> {
    if let Some(relative) = artifact.source_path.as_deref() {
        let root = fs::canonicalize(manifest_dir)?;
        let source = fs::canonicalize(root.join(relative)).map_err(|error| {
            SkillSdkError::new(
                "prebuilt_source_missing",
                format!("path={relative} error={error}"),
            )
            .phase("dependencies")
        })?;
        if !source.starts_with(&root) || !source.is_file() {
            return Err(
                SkillSdkError::new("prebuilt_source_escape", source.display().to_string())
                    .phase("dependencies"),
            );
        }
        verify_source(&source, artifact)?;
        return Ok(source);
    }
    let url = artifact.url.as_deref().ok_or_else(|| {
        SkillSdkError::new("prebuilt_source_missing", "source_path or url is required")
            .phase("dependencies")
    })?;
    let cache = cache_root.join("downloads");
    fs::create_dir_all(&cache)?;
    let destination = cache.join(artifact.sha256.to_ascii_lowercase());
    if destination.is_file() && verify_source(&destination, artifact).is_ok() {
        return Ok(destination);
    }
    if !allow_network {
        return Err(SkillSdkError::new(
            "prebuilt_download_cache_required",
            format!("sha256={}", artifact.sha256),
        )
        .phase("dependencies"));
    }
    download_https(url, &destination, artifact)?;
    Ok(destination)
}

pub(crate) fn install(
    source: &Path,
    artifact: &PlatformArtifact,
    staging_root: &Path,
    entrypoint: &str,
) -> SkillSdkResult<PathBuf> {
    match artifact.archive {
        Some(ArchiveFormat::Zip) => extract_zip(source, staging_root)?,
        Some(ArchiveFormat::TarGz) => extract_tar_gz(source, staging_root)?,
        None => {
            let destination = staging_root.join(entrypoint);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source, &destination)?;
        }
    }
    let destination = staging_root.join(entrypoint);
    let canonical_root = fs::canonicalize(staging_root)?;
    let canonical_destination = fs::canonicalize(&destination).map_err(|error| {
        SkillSdkError::new(
            "prebuilt_entrypoint_missing",
            format!("path={entrypoint} error={error}"),
        )
        .phase("artifact")
    })?;
    if !canonical_destination.starts_with(&canonical_root) || !canonical_destination.is_file() {
        return Err(SkillSdkError::new(
            "prebuilt_entrypoint_escape",
            canonical_destination.display().to_string(),
        )
        .phase("artifact"));
    }
    if artifact.executable {
        set_executable(&canonical_destination)?;
    }
    Ok(canonical_destination)
}

fn verify_source(path: &Path, artifact: &PlatformArtifact) -> SkillSdkResult<()> {
    let actual_size = fs::metadata(path)?.len();
    if actual_size > MAX_PREBUILT_DOWNLOAD_BYTES {
        return Err(SkillSdkError::new(
            "prebuilt_size_limit_exceeded",
            format!("limit={MAX_PREBUILT_DOWNLOAD_BYTES} actual={actual_size}"),
        )
        .phase("artifact"));
    }
    if artifact
        .size_bytes
        .is_some_and(|expected| expected != actual_size)
    {
        return Err(SkillSdkError::new(
            "prebuilt_size_mismatch",
            format!(
                "expected={} actual={actual_size}",
                artifact.size_bytes.unwrap_or_default()
            ),
        )
        .phase("artifact"));
    }
    if digest_file(path)? != artifact.sha256.to_ascii_lowercase() {
        return Err(SkillSdkError::new(
            "prebuilt_digest_mismatch",
            format!("source={}", path.display()),
        )
        .phase("artifact"));
    }
    Ok(())
}

fn download_https(
    url: &str,
    destination: &Path,
    artifact: &PlatformArtifact,
) -> SkillSdkResult<()> {
    if !url.starts_with("https://") {
        return Err(SkillSdkError::new(
            "prebuilt_url_insecure",
            "only HTTPS downloads are allowed",
        )
        .phase("dependencies"));
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && attempt.url().scheme() == "https" {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|error| {
            SkillSdkError::new("prebuilt_download_client_failed", error.to_string())
                .phase("dependencies")
        })?;
    let mut response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "agent-skill-sdk/1")
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| {
            SkillSdkError::new("prebuilt_download_failed", error.to_string())
                .phase("dependencies")
                .retryable(true)
        })?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_PREBUILT_DOWNLOAD_BYTES)
    {
        return Err(SkillSdkError::new(
            "prebuilt_size_limit_exceeded",
            format!("limit={MAX_PREBUILT_DOWNLOAD_BYTES}"),
        )
        .phase("dependencies"));
    }
    let temporary = destination.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = response.read(&mut buffer).map_err(|error| {
            SkillSdkError::new("prebuilt_download_read_failed", error.to_string())
                .phase("dependencies")
        })?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > MAX_PREBUILT_DOWNLOAD_BYTES {
            let _ = fs::remove_file(&temporary);
            return Err(SkillSdkError::new(
                "prebuilt_size_limit_exceeded",
                format!("limit={MAX_PREBUILT_DOWNLOAD_BYTES}"),
            )
            .phase("dependencies"));
        }
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    drop(output);
    let actual_digest = hex::encode(digest.finalize());
    let expected_size = artifact.size_bytes.unwrap_or_default();
    if total != expected_size || actual_digest != artifact.sha256.to_ascii_lowercase() {
        let _ = fs::remove_file(&temporary);
        return Err(SkillSdkError::new(
            "prebuilt_download_verification_failed",
            format!("expected_size={expected_size} actual_size={total}"),
        )
        .phase("artifact"));
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn extract_zip(source: &Path, destination: &Path) -> SkillSdkResult<()> {
    let file = File::open(source)?;
    let mut archive = zip::ZipArchive::new(file).map_err(archive_error)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(archive_limit("entry count"));
    }
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(archive_error)?;
        let relative = entry.enclosed_name().ok_or_else(archive_path_error)?;
        validate_archive_path(&relative)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(archive_link_error());
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err(archive_limit("expanded size"));
        }
        let target = safe_archive_destination(destination, &relative)?;
        if entry.is_dir() {
            fs::create_dir_all(target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = create_archive_file(&target)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn extract_tar_gz(source: &Path, destination: &Path) -> SkillSdkResult<()> {
    let decoder = GzDecoder::new(File::open(source)?);
    let mut archive = tar::Archive::new(decoder);
    let mut count = 0_usize;
    let mut expanded = 0_u64;
    for entry in archive.entries().map_err(archive_error)? {
        let mut entry = entry.map_err(archive_error)?;
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(archive_limit("entry count"));
        }
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(archive_link_error());
        }
        let relative = entry.path().map_err(archive_error)?.into_owned();
        validate_archive_path(&relative)?;
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err(archive_limit("expanded size"));
        }
        let target = safe_archive_destination(destination, &relative)?;
        if entry_type.is_dir() {
            fs::create_dir_all(target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = create_archive_file(&target)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn validate_archive_path(path: &Path) -> SkillSdkResult<()> {
    let mut components = path.components();
    let starts_in_runtime =
        matches!(components.next(), Some(Component::Normal(value)) if value == "runtime");
    let remaining_safe = components.all(|component| matches!(component, Component::Normal(_)));
    if !starts_in_runtime || !remaining_safe {
        return Err(archive_path_error());
    }
    Ok(())
}

fn safe_archive_destination(root: &Path, relative: &Path) -> SkillSdkResult<PathBuf> {
    validate_archive_path(relative)?;
    let target = root.join(relative);
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(archive_path_error());
        };
        current.push(value);
        if fs::symlink_metadata(&current)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(archive_link_error());
        }
    }
    Ok(target)
}

fn create_archive_file(path: &Path) -> SkillSdkResult<File> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            SkillSdkError::new(
                "prebuilt_archive_collision",
                format!("path={} error={error}", path.display()),
            )
            .phase("artifact")
        })
}

fn archive_error(error: impl std::fmt::Display) -> SkillSdkError {
    SkillSdkError::new("prebuilt_archive_invalid", error.to_string()).phase("artifact")
}

fn archive_path_error() -> SkillSdkError {
    SkillSdkError::new(
        "prebuilt_archive_path_unsafe",
        "archive entries must stay under runtime/",
    )
    .phase("artifact")
}

fn archive_link_error() -> SkillSdkError {
    SkillSdkError::new(
        "prebuilt_archive_link_forbidden",
        "archive links and special files are forbidden",
    )
    .phase("artifact")
}

fn archive_limit(kind: &str) -> SkillSdkError {
    SkillSdkError::new(
        "prebuilt_archive_limit_exceeded",
        format!("limit_kind={kind}"),
    )
    .phase("artifact")
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

#[cfg(test)]
#[path = "prebuilt_tests.rs"]
mod tests;
