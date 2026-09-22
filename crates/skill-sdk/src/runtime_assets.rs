use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::Path;

use crate::manifest::{validate_relative_path, PackageManifest, RuntimeFile};
use crate::receipt::{digest_file, ArtifactReceipt};
use crate::{SkillSdkError, SkillSdkResult};

const MAX_FILES: usize = 50_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) fn validate(files: &[RuntimeFile]) -> SkillSdkResult<()> {
    if files.len() > 128 {
        return Err(SkillSdkError::new("runtime_assets_limit", "declarations"));
    }
    for (index, file) in files.iter().enumerate() {
        validate_relative_path(&file.source, "runtime_files.source", false)?;
        validate_relative_path(&file.destination, "runtime_files.destination", false)?;
        let destination = Path::new(&file.destination);
        if !destination.starts_with("runtime/assets") || destination == Path::new("runtime/assets")
        {
            return Err(SkillSdkError::new(
                "runtime_asset_destination_invalid",
                &file.destination,
            ));
        }
        if files[..index].iter().any(|other| {
            destination.starts_with(&other.destination)
                || Path::new(&other.destination).starts_with(destination)
        }) {
            return Err(SkillSdkError::new(
                "runtime_asset_destination_overlap",
                &file.destination,
            ));
        }
    }
    Ok(())
}

pub(crate) fn install(
    manifest: &PackageManifest,
    source_root: &Path,
    staging: &Path,
    artifacts: &mut Vec<ArtifactReceipt>,
) -> SkillSdkResult<()> {
    validate(&manifest.build.runtime_files)?;
    let mut budget = (0_usize, 0_u64);
    for file in &manifest.build.runtime_files {
        reject_symlink_components(source_root, Path::new(&file.source))?;
        reject_symlink_components(staging, Path::new(&file.destination))?;
        copy_entry(
            &source_root.join(&file.source),
            &staging.join(&file.destination),
            staging,
            artifacts,
            &mut budget,
        )?;
    }
    Ok(())
}

fn reject_symlink_components(root: &Path, relative: &Path) -> SkillSdkResult<()> {
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(SkillSdkError::new(
                    "runtime_asset_symlink_forbidden",
                    current.display().to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn copy_entry(
    source: &Path,
    destination: &Path,
    staging: &Path,
    artifacts: &mut Vec<ArtifactReceipt>,
    budget: &mut (usize, u64),
) -> SkillSdkResult<()> {
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        SkillSdkError::new(
            "runtime_asset_unavailable",
            format!("path={} error={error}", source.display()),
        )
    })?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(SkillSdkError::new(
            "runtime_asset_type_invalid",
            source.display().to_string(),
        ));
    }
    budget.0 += 1;
    budget.1 = budget.1.saturating_add(if metadata.is_file() {
        metadata.len()
    } else {
        0
    });
    if budget.0 > MAX_FILES || budget.1 > MAX_BYTES {
        return Err(SkillSdkError::new(
            "runtime_assets_limit",
            source.display().to_string(),
        ));
    }
    if destination.exists() {
        return Err(SkillSdkError::new(
            "runtime_asset_collision",
            destination.display().to_string(),
        ));
    }
    fs::create_dir_all(
        destination
            .parent()
            .ok_or_else(|| SkillSdkError::new("runtime_asset_destination_invalid", "parent"))?,
    )?;
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        let remaining = MAX_FILES.saturating_sub(budget.0);
        let mut entries = fs::read_dir(source)?
            .take(remaining + 1)
            .collect::<Result<Vec<_>, _>>()?;
        if entries.len() > remaining {
            return Err(SkillSdkError::new("runtime_assets_limit", "entries"));
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            copy_entry(
                &entry.path(),
                &destination.join(entry.file_name()),
                staging,
                artifacts,
                budget,
            )?;
        }
    } else {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let input = options.open(source)?;
        let opened = input.metadata()?;
        if !opened.is_file() || opened.len() != metadata.len() {
            return Err(SkillSdkError::new("runtime_asset_changed", "metadata"));
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let copied = std::io::copy(&mut input.take(metadata.len() + 1), &mut output)?;
        if copied != metadata.len() {
            return Err(SkillSdkError::new(
                "runtime_asset_changed",
                source.display().to_string(),
            ));
        }
        artifacts.push(ArtifactReceipt {
            path: destination
                .strip_prefix(staging)
                .map_err(|_| SkillSdkError::new("runtime_asset_path_escape", "destination"))?
                .to_string_lossy()
                .replace('\\', "/"),
            sha256: digest_file(destination)?,
            size_bytes: copied,
            executable: false,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_assets_tests.rs"]
mod tests;
