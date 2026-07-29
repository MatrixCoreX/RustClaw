use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};

use crate::manifest::ArchiveFormat;
use crate::{ArtifactSpill, BoundedResult, SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeArchiveLimits {
    pub max_entries: usize,
    pub max_expanded_bytes: u64,
    pub max_depth: usize,
    pub max_elapsed: Duration,
}

impl SafeArchiveLimits {
    pub fn adaptive_for(source: &Path, destination_parent: &Path) -> SkillSdkResult<Self> {
        let compressed_bytes = source.metadata()?.len();
        let available_bytes = fs2::available_space(destination_parent).map_err(|error| {
            SkillSdkError::new("archive_disk_budget_unavailable", error.to_string())
                .phase("archive_preflight")
        })?;
        let expansion_budget = compressed_bytes
            .saturating_mul(512)
            .max(8 * 1024 * 1024 * 1024)
            .min(256 * 1024 * 1024 * 1024)
            .min(available_bytes.saturating_mul(8) / 10);
        if expansion_budget < compressed_bytes {
            return Err(SkillSdkError::new(
                "archive_disk_budget_insufficient",
                format!("compressed_bytes={compressed_bytes} available_bytes={available_bytes}"),
            )
            .phase("archive_preflight"));
        }
        Ok(Self {
            max_entries: ((compressed_bytes / 64) as usize).clamp(100_000, 2_000_000),
            max_expanded_bytes: expansion_budget,
            max_depth: 256,
            max_elapsed: Duration::from_secs(300),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeArchiveEntry {
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeArchiveInspection {
    pub format: String,
    pub entry_count: usize,
    pub expanded_bytes: u64,
    pub entries: Vec<SafeArchiveEntry>,
}

pub fn inspect_safe_archive(
    source: &Path,
    limits: SafeArchiveLimits,
) -> SkillSdkResult<SafeArchiveInspection> {
    let format = format_from_path(source)?;
    let started = Instant::now();
    match format {
        ArchiveFormat::Zip => inspect_zip(source, limits, started),
        ArchiveFormat::TarGz => inspect_tar_gz(source, limits, started),
    }
}

pub fn extract_safe_archive(
    source: &Path,
    destination: &Path,
    limits: SafeArchiveLimits,
) -> SkillSdkResult<SafeArchiveInspection> {
    if destination.exists() {
        return Err(SkillSdkError::new(
            "archive_destination_exists",
            format!("path={}", destination.display()),
        )
        .phase("archive_extract"));
    }
    let inspection = inspect_safe_archive(source, limits)?;
    fs::create_dir(destination)?;
    let started = Instant::now();
    let extraction = match format_from_path(source)? {
        ArchiveFormat::Zip => extract_zip(source, destination, limits, started),
        ArchiveFormat::TarGz => extract_tar_gz(source, destination, limits, started),
    };
    if let Err(error) = extraction {
        let _ = fs::remove_dir_all(destination);
        return Err(error);
    }
    Ok(inspection)
}

pub fn read_safe_archive_member(
    source: &Path,
    member: &str,
    limits: SafeArchiveLimits,
    inline_bytes: usize,
    spill: Option<&ArtifactSpill>,
) -> SkillSdkResult<BoundedResult<String>> {
    let member_path = Path::new(member);
    validate_relative_path(member_path, limits.max_depth)?;
    let inspection = inspect_safe_archive(source, limits)?;
    let matches = inspection
        .entries
        .iter()
        .filter(|entry| entry.path == member)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(
            SkillSdkError::new("archive_member_not_found", format!("member={member}"))
                .phase("archive_read"),
        );
    }
    if matches.len() > 1 {
        return Err(SkillSdkError::new(
            "archive_member_ambiguous",
            format!("member={member} matches={}", matches.len()),
        )
        .phase("archive_read"));
    }
    if matches[0].kind != "file" {
        return Err(
            SkillSdkError::new("archive_member_not_file", format!("member={member}"))
                .phase("archive_read"),
        );
    }
    match format_from_path(source)? {
        ArchiveFormat::Zip => {
            let mut archive = zip::ZipArchive::new(File::open(source)?).map_err(archive_invalid)?;
            for index in 0..archive.len() {
                let entry = archive.by_index(index).map_err(archive_invalid)?;
                if entry
                    .enclosed_name()
                    .is_some_and(|path| path == member_path)
                {
                    return bounded_member_reader(
                        entry,
                        matches[0].size_bytes,
                        inline_bytes,
                        spill,
                    );
                }
            }
        }
        ArchiveFormat::TarGz => {
            let decoder = GzDecoder::new(File::open(source)?);
            let mut archive = tar::Archive::new(decoder);
            for entry in archive.entries().map_err(archive_invalid)? {
                let entry = entry.map_err(archive_invalid)?;
                if entry.path().map_err(archive_invalid)?.as_ref() == member_path {
                    return bounded_member_reader(
                        entry,
                        matches[0].size_bytes,
                        inline_bytes,
                        spill,
                    );
                }
            }
        }
    }
    Err(
        SkillSdkError::new("archive_member_not_found", format!("member={member}"))
            .phase("archive_read"),
    )
}

fn bounded_member_reader<R: Read>(
    mut reader: R,
    size_bytes: u64,
    inline_bytes: usize,
    spill: Option<&ArtifactSpill>,
) -> SkillSdkResult<BoundedResult<String>> {
    if size_bytes <= inline_bytes as u64 {
        let mut bytes = Vec::with_capacity(size_bytes as usize);
        reader.read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        let size = bytes.len() as u64;
        return Ok(BoundedResult::complete(text).with_sizes(size, size));
    }
    let spill = spill.ok_or_else(|| {
        SkillSdkError::new(
            "bounded_result_recovery_unavailable",
            "large archive member requires declared skill storage",
        )
        .phase("archive_read")
    })?;
    spill.spill_text_reader(
        "archive-member",
        "application/octet-stream",
        reader,
        inline_bytes,
    )
}

fn inspect_zip(
    source: &Path,
    limits: SafeArchiveLimits,
    started: Instant,
) -> SkillSdkResult<SafeArchiveInspection> {
    let mut archive = zip::ZipArchive::new(File::open(source)?).map_err(archive_invalid)?;
    let mut entries = Vec::with_capacity(archive.len().min(limits.max_entries));
    let mut expanded_bytes = 0_u64;
    for index in 0..archive.len() {
        check_budget(started, entries.len() + 1, expanded_bytes, limits)?;
        let entry = archive.by_index(index).map_err(archive_invalid)?;
        let relative = entry.enclosed_name().ok_or_else(unsafe_archive_path)?;
        validate_relative_path(&relative, limits.max_depth)?;
        let kind = zip_entry_kind(&entry)?;
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| archive_limit("expanded_bytes_overflow"))?;
        check_budget(started, entries.len() + 1, expanded_bytes, limits)?;
        entries.push(SafeArchiveEntry {
            path: relative.to_string_lossy().to_string(),
            kind: kind.to_string(),
            size_bytes: entry.size(),
        });
    }
    Ok(inspection("zip", entries, expanded_bytes))
}

fn inspect_tar_gz(
    source: &Path,
    limits: SafeArchiveLimits,
    started: Instant,
) -> SkillSdkResult<SafeArchiveInspection> {
    let decoder = GzDecoder::new(File::open(source)?);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = Vec::new();
    let mut expanded_bytes = 0_u64;
    for entry in archive.entries().map_err(archive_invalid)? {
        check_budget(started, entries.len() + 1, expanded_bytes, limits)?;
        let entry = entry.map_err(archive_invalid)?;
        let entry_type = entry.header().entry_type();
        let kind = if entry_type.is_file() {
            "file"
        } else if entry_type.is_dir() {
            "directory"
        } else {
            return Err(forbidden_archive_type("tar special/link entry"));
        };
        let relative = entry.path().map_err(archive_invalid)?.into_owned();
        validate_relative_path(&relative, limits.max_depth)?;
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| archive_limit("expanded_bytes_overflow"))?;
        check_budget(started, entries.len() + 1, expanded_bytes, limits)?;
        entries.push(SafeArchiveEntry {
            path: relative.to_string_lossy().to_string(),
            kind: kind.to_string(),
            size_bytes: entry.size(),
        });
    }
    Ok(inspection("tar_gz", entries, expanded_bytes))
}

fn extract_zip(
    source: &Path,
    destination: &Path,
    limits: SafeArchiveLimits,
    started: Instant,
) -> SkillSdkResult<()> {
    let mut archive = zip::ZipArchive::new(File::open(source)?).map_err(archive_invalid)?;
    let mut expanded_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(archive_invalid)?;
        let relative = entry.enclosed_name().ok_or_else(unsafe_archive_path)?;
        validate_relative_path(&relative, limits.max_depth)?;
        let kind = zip_entry_kind(&entry)?;
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| archive_limit("expanded_bytes_overflow"))?;
        check_budget(started, index + 1, expanded_bytes, limits)?;
        let target = safe_destination(destination, &relative)?;
        if kind == "directory" {
            fs::create_dir_all(target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = create_new_file(&target)?;
        io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn extract_tar_gz(
    source: &Path,
    destination: &Path,
    limits: SafeArchiveLimits,
    started: Instant,
) -> SkillSdkResult<()> {
    let decoder = GzDecoder::new(File::open(source)?);
    let mut archive = tar::Archive::new(decoder);
    let mut count = 0_usize;
    let mut expanded_bytes = 0_u64;
    for entry in archive.entries().map_err(archive_invalid)? {
        let mut entry = entry.map_err(archive_invalid)?;
        count += 1;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(forbidden_archive_type("tar special/link entry"));
        }
        let relative = entry.path().map_err(archive_invalid)?.into_owned();
        validate_relative_path(&relative, limits.max_depth)?;
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| archive_limit("expanded_bytes_overflow"))?;
        check_budget(started, count, expanded_bytes, limits)?;
        let target = safe_destination(destination, &relative)?;
        if entry_type.is_dir() {
            fs::create_dir_all(target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = create_new_file(&target)?;
        io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn inspection(
    format: &str,
    entries: Vec<SafeArchiveEntry>,
    expanded_bytes: u64,
) -> SafeArchiveInspection {
    SafeArchiveInspection {
        format: format.to_string(),
        entry_count: entries.len(),
        expanded_bytes,
        entries,
    }
}

fn format_from_path(path: &Path) -> SkillSdkResult<ArchiveFormat> {
    let name = path.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".zip") {
        Ok(ArchiveFormat::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Ok(ArchiveFormat::TarGz)
    } else {
        Err(unsupported_format(path))
    }
}

fn validate_relative_path(path: &Path, max_depth: usize) -> SkillSdkResult<()> {
    let mut depth = 0_usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_archive_path());
            }
        }
    }
    if depth == 0 || depth > max_depth {
        return Err(archive_limit("path_depth"));
    }
    Ok(())
}

fn zip_entry_kind(entry: &zip::read::ZipFile<'_>) -> SkillSdkResult<&'static str> {
    if entry.is_dir() {
        return Ok("directory");
    }
    if entry.unix_mode().is_some_and(|mode| {
        let kind = mode & 0o170000;
        kind != 0 && kind != 0o100000
    }) {
        return Err(forbidden_archive_type("zip special/link entry"));
    }
    Ok("file")
}

fn safe_destination(root: &Path, relative: &Path) -> SkillSdkResult<PathBuf> {
    validate_relative_path(relative, usize::MAX)?;
    let target = root.join(relative);
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        current.push(value);
        if current
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(forbidden_archive_type("destination symlink"));
        }
    }
    Ok(target)
}

fn create_new_file(path: &Path) -> SkillSdkResult<File> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            SkillSdkError::new(
                "archive_destination_collision",
                format!("path={} error={error}", path.display()),
            )
            .phase("archive_extract")
        })
}

fn check_budget(
    started: Instant,
    entries: usize,
    expanded_bytes: u64,
    limits: SafeArchiveLimits,
) -> SkillSdkResult<()> {
    if entries > limits.max_entries {
        return Err(archive_limit("entry_count"));
    }
    if expanded_bytes > limits.max_expanded_bytes {
        return Err(archive_limit("expanded_bytes"));
    }
    if started.elapsed() > limits.max_elapsed {
        return Err(archive_limit("elapsed_time"));
    }
    Ok(())
}

fn unsupported_format(path: &Path) -> SkillSdkError {
    SkillSdkError::new(
        "archive_format_unsupported",
        format!("path={}", path.display()),
    )
    .phase("archive_preflight")
}

fn archive_invalid(error: impl std::fmt::Display) -> SkillSdkError {
    SkillSdkError::new("archive_invalid", error.to_string()).phase("archive_preflight")
}

fn unsafe_archive_path() -> SkillSdkError {
    SkillSdkError::new(
        "archive_path_unsafe",
        "archive member path is absolute or contains traversal",
    )
    .phase("archive_preflight")
}

fn forbidden_archive_type(detail: &str) -> SkillSdkError {
    SkillSdkError::new("archive_entry_type_forbidden", detail).phase("archive_preflight")
}

fn archive_limit(kind: &str) -> SkillSdkError {
    SkillSdkError::new("archive_budget_exceeded", format!("limit_kind={kind}"))
        .phase("archive_preflight")
}

#[cfg(test)]
#[path = "safe_archive_tests.rs"]
mod tests;
