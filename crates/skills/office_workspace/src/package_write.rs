use crate::error::{OfficeError, OfficeResult};
use crate::model::OfficeFormat;
use crate::package::{resolve_input_path, OfficePackage};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Clone, Debug)]
pub struct PublishEvidence {
    pub output_path: PathBuf,
    pub output_sha256: String,
    pub backup_path: Option<PathBuf>,
}

pub fn resolve_output_path(value: &str) -> OfficeResult<PathBuf> {
    let path = Path::new(value);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(OfficeError::new(
            "path_traversal",
            "parent path traversal is not allowed",
            json!({"path": value}),
        ));
    }
    let path = resolve_input_path(value)?;
    if path.file_name().is_none() {
        return Err(OfficeError::invalid("output path must name a file"));
    }
    Ok(path)
}

pub fn publish_package(
    members: &BTreeMap<String, Vec<u8>>,
    output_path: &Path,
    format: OfficeFormat,
    overwrite: bool,
    in_place_source: Option<&Path>,
    source_hash: Option<&str>,
) -> OfficeResult<PublishEvidence> {
    if output_path.exists() && !overwrite {
        return Err(OfficeError::new(
            "output_exists",
            "output path already exists and overwrite was not approved",
            json!({"output_path": output_path.display().to_string()}),
        ));
    }
    if let Some(source) = in_place_source {
        if source != output_path {
            return Err(OfficeError::new(
                "invalid_in_place_target",
                "in-place mutation requires source and output paths to match",
                json!({
                    "source_path": source.display().to_string(),
                    "output_path": output_path.display().to_string()
                }),
            ));
        }
    }
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        OfficeError::new(
            "transaction_failed",
            format!("cannot create output directory: {error}"),
            json!({"directory": parent.display().to_string()}),
        )
    })?;
    let file_name = output_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("office-output");
    let temp_path = parent.join(format!(".{file_name}.rustclaw-{}.tmp", Uuid::new_v4()));
    let result = write_and_validate(members, &temp_path, format);
    let package = match result {
        Ok(package) => package,
        Err(error) => {
            fs::remove_file(&temp_path).ok();
            return Err(error);
        }
    };

    let backup_path = if let Some(source) = in_place_source {
        let hash = source_hash.unwrap_or("unknown");
        let backup = parent.join(format!(
            ".{file_name}.rustclaw-backup-{}",
            &hash[..hash.len().min(16)]
        ));
        fs::copy(source, &backup).map_err(|error| {
            fs::remove_file(&temp_path).ok();
            OfficeError::new(
                "backup_failed",
                format!("cannot create in-place backup: {error}"),
                json!({"backup_path": backup.display().to_string()}),
            )
        })?;
        Some(backup)
    } else {
        None
    };

    if let Err(error) = fs::rename(&temp_path, output_path) {
        fs::remove_file(&temp_path).ok();
        return Err(OfficeError::new(
            "atomic_publish_failed",
            format!("cannot atomically publish Office artifact: {error}"),
            json!({
                "temp_path": temp_path.display().to_string(),
                "output_path": output_path.display().to_string(),
                "backup_path": backup_path.as_ref().map(|path| path.display().to_string())
            }),
        ));
    }
    sync_parent(parent);
    Ok(PublishEvidence {
        output_path: output_path.to_path_buf(),
        output_sha256: package.source.sha256,
        backup_path,
    })
}

fn write_and_validate(
    members: &BTreeMap<String, Vec<u8>>,
    temp_path: &Path,
    format: OfficeFormat,
) -> OfficeResult<OfficePackage> {
    let file = fs::File::create(temp_path).map_err(|error| {
        OfficeError::new(
            "transaction_failed",
            format!("cannot create temporary Office package: {error}"),
            json!({"temp_path": temp_path.display().to_string()}),
        )
    })?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, bytes) in members {
        zip.start_file(name, options).map_err(|error| {
            OfficeError::new(
                "transaction_failed",
                format!("cannot start Office package member: {error}"),
                json!({"member": name}),
            )
        })?;
        zip.write_all(bytes).map_err(|error| {
            OfficeError::new(
                "transaction_failed",
                format!("cannot write Office package member: {error}"),
                json!({"member": name}),
            )
        })?;
    }
    let mut file = zip.finish().map_err(|error| {
        OfficeError::new(
            "transaction_failed",
            format!("cannot finish Office package: {error}"),
            json!({}),
        )
    })?;
    file.flush().map_err(|error| {
        OfficeError::new(
            "transaction_failed",
            format!("cannot flush Office package: {error}"),
            json!({}),
        )
    })?;
    file.sync_all().map_err(|error| {
        OfficeError::new(
            "transaction_failed",
            format!("cannot sync Office package: {error}"),
            json!({}),
        )
    })?;
    OfficePackage::open(temp_path, Some(format)).map_err(|error| {
        OfficeError::new(
            "validation_failed",
            "reopening the temporary Office package failed",
            json!({"cause": error.code, "details": error.details}),
        )
    })
}

#[cfg(unix)]
fn sync_parent(path: &Path) {
    if let Ok(directory) = fs::File::open(path) {
        directory.sync_all().ok();
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}

#[cfg(test)]
#[path = "package_write_tests.rs"]
mod tests;
