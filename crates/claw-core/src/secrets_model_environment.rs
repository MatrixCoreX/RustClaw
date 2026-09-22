//! UI-managed literal environment assignments. Never evaluate this file as shell code.
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{SecretValue, SecretsError};

const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_VALUE_BYTES: usize = 4096;

pub fn path(workspace: &Path) -> PathBuf {
    crate::git_remote_config::git_credential_store_path(workspace).with_file_name("models.env")
}

fn environment_name(vendor: &str) -> Option<&'static str> {
    crate::config::llm_vendor_api_key_env_names(vendor)
        .last()
        .copied()
}

fn failure(message: &str) -> SecretsError {
    SecretsError::BackendIo {
        name: "model_environment".into(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, message),
    }
}

fn io_error(source: std::io::Error) -> SecretsError {
    SecretsError::BackendIo {
        name: "model_environment".into(),
        source,
    }
}

fn validate_path(path: &Path) -> Result<(), SecretsError> {
    // The workspace itself may be a legitimate platform symlink (/tmp on macOS).
    for ancestor in path.ancestors().take(3) {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(failure("model_environment_symlink_rejected"))
            }
            Ok(_) => (),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (),
            Err(err) => return Err(io_error(err)),
        }
    }
    Ok(())
}

fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_VALUE_BYTES
        && !value.chars().any(char::is_control)
        && !value.starts_with("REPLACE_ME")
}

fn read(path: &Path) -> Result<BTreeMap<String, String>, SecretsError> {
    validate_path(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => return Err(io_error(err)),
    };
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(failure("model_environment_file_invalid"));
    }
    let mut raw = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(io_error)?;
    if raw.len() as u64 > MAX_FILE_BYTES {
        return Err(failure("model_environment_file_too_large"));
    }
    let mut entries = BTreeMap::new();
    for line in raw
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| failure("model_environment_assignment_invalid"))?;
        let vendor = name
            .strip_suffix("_API_KEY")
            .unwrap_or("")
            .to_ascii_lowercase();
        if environment_name(&vendor) != Some(name)
            || !valid_value(value)
            || entries.insert(name.into(), value.into()).is_some()
        {
            return Err(failure("model_environment_assignment_invalid"));
        }
    }
    Ok(entries)
}

pub fn lookup_at(path: &Path, vendor: &str) -> Result<Option<SecretValue>, SecretsError> {
    let Some(name) = environment_name(vendor) else {
        return Ok(None);
    };
    Ok(read(path)?.remove(name).map(SecretValue::new))
}

pub fn save(workspace: &Path, vendor: &str, value: &str) -> Result<(), SecretsError> {
    let name =
        environment_name(vendor).ok_or_else(|| failure("model_environment_vendor_invalid"))?;
    if !valid_value(value) {
        return Err(failure("model_environment_value_invalid"));
    }
    let path = path(workspace);
    validate_path(&path)?;
    let _lock = super::file::lock_document(&path, "model_environment")?;
    let mut entries = read(&path)?;
    entries.insert(name.into(), value.into());
    let mut payload =
        String::from("# Managed model credentials. Literal assignments; do not source as shell.\n");
    for (key, value) in entries {
        payload.push_str(&format!("{key}={value}\n"));
    }
    let temporary = path.with_file_name(format!(".models-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary).map_err(io_error)?;
        file.write_all(payload.as_bytes()).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&temporary, &path).map_err(io_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
#[path = "secrets_model_environment_tests.rs"]
mod tests;
