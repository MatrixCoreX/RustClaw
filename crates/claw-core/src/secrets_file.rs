use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{validate_secret_name, EnvSecretsBroker, SecretValue, SecretsBroker, SecretsError};

const FILE_SECRET_SCHEMA_VERSION: u32 = 1;
const MAX_SECRET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretProtectionSource {
    Environment,
    SystemdCredential,
    MacosKeychain,
    PrivateFile,
}

impl SecretProtectionSource {
    pub fn machine_name(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::SystemdCredential => "systemd_credential",
            Self::MacosKeychain => "macos_keychain",
            Self::PrivateFile => "private_file_fallback",
        }
    }
}

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

    pub fn lookup_with_source(
        &self,
        name: &str,
    ) -> Result<Option<(SecretValue, SecretProtectionSource)>, SecretsError> {
        validate_secret_name(name)?;
        if let Some(value) = self.environment.lookup(name)? {
            return Ok(Some((value, SecretProtectionSource::Environment)));
        }
        if let Some(value) = lookup_systemd_credential(name)? {
            return Ok(Some((value, SecretProtectionSource::SystemdCredential)));
        }
        if let Some(value) = lookup_macos_keychain(name)? {
            return Ok(Some((value, SecretProtectionSource::MacosKeychain)));
        }
        let document = read_document(&self.path, name)?;
        Ok(document
            .secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .map(|value| (SecretValue::new(value), SecretProtectionSource::PrivateFile)))
    }
}

impl SecretsBroker for EnvFileSecretsBroker {
    fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
        Ok(self.lookup_with_source(name)?.map(|(value, _)| value))
    }

    fn label(&self) -> &str {
        "environment+os_broker+private_file_fallback"
    }
}

fn lookup_systemd_credential(name: &str) -> Result<Option<SecretValue>, SecretsError> {
    let Some(directory) = std::env::var_os("CREDENTIALS_DIRECTORY") else {
        return Ok(None);
    };
    let directory = PathBuf::from(directory);
    let path = directory.join(name);
    reject_symlink(&path, name)?;
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SecretsError::BackendIo {
                name: name.to_string(),
                source,
            })
        }
    };
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SECRET_BYTES as u64 {
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("systemd_credential_file_invalid"),
        });
    }
    let raw = fs::read_to_string(&path).map_err(|source| SecretsError::BackendIo {
        name: name.to_string(),
        source,
    })?;
    let value = raw.strip_suffix('\n').unwrap_or(&raw);
    if value.is_empty() || value.contains('\0') {
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("systemd_credential_value_invalid"),
        });
    }
    Ok(Some(SecretValue::new(value)))
}

#[cfg(target_os = "macos")]
fn lookup_macos_keychain(name: &str) -> Result<Option<SecretValue>, SecretsError> {
    let service = format!("agent-runtime/{name}");
    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", &service, "-w"])
        .output()
        .map_err(|source| SecretsError::BackendIo {
            name: name.to_string(),
            source,
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    if output.stdout.is_empty() || output.stdout.len() > MAX_SECRET_BYTES {
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("macos_keychain_value_invalid"),
        });
    }
    let raw = String::from_utf8(output.stdout).map_err(|error| SecretsError::BackendIo {
        name: name.to_string(),
        source: invalid_data(format!("macos_keychain_value_not_utf8:{error}")),
    })?;
    let value = raw.strip_suffix('\n').unwrap_or(&raw);
    if value.is_empty() || value.contains('\0') {
        return Err(SecretsError::BackendIo {
            name: name.to_string(),
            source: invalid_data("macos_keychain_value_invalid"),
        });
    }
    Ok(Some(SecretValue::new(value)))
}

#[cfg(not(target_os = "macos"))]
fn lookup_macos_keychain(_name: &str) -> Result<Option<SecretValue>, SecretsError> {
    Ok(None)
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
