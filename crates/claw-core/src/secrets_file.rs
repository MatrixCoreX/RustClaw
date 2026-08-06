use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{validate_secret_name, EnvSecretsBroker, SecretValue, SecretsBroker, SecretsError};

const FILE_SECRET_SCHEMA_VERSION: u32 = 1;
const MAX_SECRET_BYTES: usize = 64 * 1024;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSecretDocument {
    schema_version: u32,
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

/// Runtime credential broker backed by a private JSON file, with deployment
/// environment variables taking precedence. The file is read for every lookup
/// so write-only UI updates and rotations take effect without restarting.
#[derive(Debug, Clone)]
pub struct EnvFileSecretsBroker {
    path: PathBuf,
    environment: EnvSecretsBroker,
}

impl EnvFileSecretsBroker {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            environment: EnvSecretsBroker::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SecretsBroker for EnvFileSecretsBroker {
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
        validate_secret_name(name)?;
        if let Some(value) = self.environment.lookup(name)? {
            return Ok(Some(value));
        }
        let document = read_document(&self.path, name)?;
        Ok(document
            .secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .map(SecretValue::new))
    }

    fn label(&self) -> &str {
        "environment+private_file"
    }
}

pub fn set_file_secret(path: &Path, name: &str, value: &str) -> Result<(), SecretsError> {
    validate_secret_name(name)?;
    if value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains('\0') {
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("secret_value_invalid"),
        });
    }
    let _lock = lock_document(path, name)?;
    let mut document = read_document(path, name)?;
    document.schema_version = FILE_SECRET_SCHEMA_VERSION;
    document.secrets.insert(name.to_string(), value.to_string());
    write_document(path, name, &document)
}

pub fn delete_file_secret(path: &Path, name: &str) -> Result<bool, SecretsError> {
    validate_secret_name(name)?;
    let _lock = lock_document(path, name)?;
    let mut document = read_document(path, name)?;
    let removed = document.secrets.remove(name).is_some();
    if removed {
        write_document(path, name, &document)?;
    }
    Ok(removed)
}

pub fn file_secret_is_configured(path: &Path, name: &str) -> Result<bool, SecretsError> {
    validate_secret_name(name)?;
    Ok(read_document(path, name)?
        .secrets
        .get(name)
        .is_some_and(|value| !value.is_empty()))
}

fn read_document(path: &Path, name: &str) -> Result<FileSecretDocument, SecretsError> {
    reject_symlink(path, name)?;
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSecretDocument {
                schema_version: FILE_SECRET_SCHEMA_VERSION,
                secrets: BTreeMap::new(),
            });
        }
        Err(source) => {
            return Err(SecretsError::BackendIo {
                name: name.to_string(),
                source,
            });
        }
    };
    let document: FileSecretDocument =
        serde_json::from_slice(&raw).map_err(|error| SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data(format!("credential_store_invalid:{error}")),
        })?;
    if document.schema_version != FILE_SECRET_SCHEMA_VERSION {
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("credential_store_schema_unsupported"),
        });
    }
    for key in document.secrets.keys() {
        validate_secret_name(key)?;
    }
    Ok(document)
}

fn write_document(
    path: &Path,
    name: &str,
    document: &FileSecretDocument,
) -> Result<(), SecretsError> {
    let parent = path.parent().ok_or_else(|| SecretsError::BackendIo {
        name: name.to_string(),
        source: invalid_data("credential_store_parent_missing"),
    })?;
    fs::create_dir_all(parent).map_err(|source| SecretsError::BackendIo {
        name: name.to_string(),
        source,
    })?;
    apply_private_directory_permissions(parent, name)?;
    reject_symlink(path, name)?;
    let payload = serde_json::to_vec(document).map_err(|error| SecretsError::BackendIo {
        name: name.to_string(),
        source: invalid_data(format!("credential_store_serialize_failed:{error}")),
    })?;
    let temp = parent.join(format!(".credentials-{}.tmp", Uuid::new_v4().simple()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|source| SecretsError::BackendIo {
            name: name.to_string(),
            source,
        })?;
    if let Err(source) = file.write_all(&payload).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source,
        });
    }
    if let Err(source) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source,
        });
    }
    apply_private_file_permissions(path, name)
}

fn lock_document(path: &Path, name: &str) -> Result<fs::File, SecretsError> {
    let parent = path.parent().ok_or_else(|| SecretsError::BackendIo {
        name: name.to_string(),
        source: invalid_data("credential_store_parent_missing"),
    })?;
    fs::create_dir_all(parent).map_err(|source| SecretsError::BackendIo {
        name: name.to_string(),
        source,
    })?;
    apply_private_directory_permissions(parent, name)?;
    let lock_path = parent.join("secrets.lock");
    reject_symlink(&lock_path, name)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options
        .open(&lock_path)
        .map_err(|source| SecretsError::BackendIo {
            name: name.to_string(),
            source,
        })?;
    fs2::FileExt::lock_exclusive(&file).map_err(|source| SecretsError::BackendIo {
        name: name.to_string(),
        source,
    })?;
    apply_private_file_permissions(&lock_path, name)?;
    Ok(file)
}

fn reject_symlink(path: &Path, name: &str) -> Result<(), SecretsError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("credential_store_symlink_rejected"),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SecretsError::BackendIo {
            name: name.to_string(),
            source,
        }),
    }
}

#[cfg(unix)]
fn apply_private_directory_permissions(path: &Path, name: &str) -> Result<(), SecretsError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
        SecretsError::BackendIo {
            name: name.to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn apply_private_directory_permissions(_path: &Path, _name: &str) -> Result<(), SecretsError> {
    Ok(())
}

#[cfg(unix)]
fn apply_private_file_permissions(path: &Path, name: &str) -> Result<(), SecretsError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        SecretsError::BackendIo {
            name: name.to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn apply_private_file_permissions(_path: &Path, _name: &str) -> Result<(), SecretsError> {
    Ok(())
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "secrets_file_tests.rs"]
mod tests;
