use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use url::Url;
use uuid::Uuid;

pub const GIT_CONNECTION_SCHEMA_VERSION: u32 = 1;
pub const GITHUB_GIT_CREDENTIAL_REF: &str = "github_git_token";
pub const GITHUB_API_CREDENTIAL_REF: &str = "github_api_token";
const MAX_PROFILES: usize = 32;
const MAX_ALLOWLIST_ITEMS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalGithubRemote {
    pub canonical_url: String,
    pub url_digest: String,
    pub host: String,
    pub owner: String,
    pub repository: String,
}

pub fn canonical_github_remote_url(raw: &str) -> anyhow::Result<CanonicalGithubRemote> {
    let parsed = Url::parse(raw).map_err(|_| anyhow!("git_remote_url_invalid"))?;
    let authority_has_explicit_port = raw
        .split_once("://")
        .and_then(|(_, remainder)| remainder.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.contains(':'));
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || authority_has_explicit_port
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(anyhow!("git_remote_url_not_allowed"));
    }
    let host = parsed
        .host_str()
        .map(|value| value.trim_end_matches('.').to_ascii_lowercase())
        .ok_or_else(|| anyhow!("git_remote_host_invalid"))?;
    if host != "github.com" {
        return Err(anyhow!("git_remote_host_not_allowed"));
    }
    let segments = parsed
        .path_segments()
        .ok_or_else(|| anyhow!("git_remote_path_invalid"))?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() != 2 || segments.iter().any(|segment| segment.contains('%')) {
        return Err(anyhow!("git_remote_path_invalid"));
    }
    let owner = normalized_repository_token(segments[0], "git_remote_owner_invalid")?;
    let repository = normalized_repository_token(
        segments[1].strip_suffix(".git").unwrap_or(segments[1]),
        "git_remote_repository_invalid",
    )?;
    let canonical_url = format!("https://{host}/{owner}/{repository}.git");
    let url_digest = format!("sha256:{:x}", Sha256::digest(canonical_url.as_bytes()));
    Ok(CanonicalGithubRemote {
        canonical_url,
        url_digest,
        host,
        owner,
        repository,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitConnectionProfile {
    pub id: String,
    pub forge_kind: String,
    pub git_host: String,
    pub api_host: String,
    pub allowed_owners: Vec<String>,
    pub allowed_repositories: Vec<String>,
    pub git_username: String,
    pub auth_scheme: String,
    pub git_credential_ref: String,
    pub api_credential_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitConnectionDocument {
    pub schema_version: u32,
    pub revision: u64,
    pub profiles: Vec<GitConnectionProfile>,
}

impl Default for GitConnectionDocument {
    fn default() -> Self {
        Self {
            schema_version: GIT_CONNECTION_SCHEMA_VERSION,
            revision: 0,
            profiles: Vec::new(),
        }
    }
}

pub fn git_connection_store_path(workspace_root: &Path) -> PathBuf {
    crate::workspace_state::workspace_state_root(workspace_root)
        .join("git")
        .join("remote-connections.json")
}

pub fn git_credential_store_path(workspace_root: &Path) -> PathBuf {
    crate::workspace_state::workspace_state_root(workspace_root)
        .join("credentials")
        .join("secrets.json")
}

pub fn load_git_connections(path: &Path) -> anyhow::Result<GitConnectionDocument> {
    reject_symlink(path)?;
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GitConnectionDocument::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut document: GitConnectionDocument =
        serde_json::from_slice(&raw).context("git_connection_store_invalid")?;
    if document.schema_version != GIT_CONNECTION_SCHEMA_VERSION {
        return Err(anyhow!("git_connection_store_schema_unsupported"));
    }
    normalize_and_validate_document(&mut document)?;
    Ok(document)
}

pub fn find_git_connection(
    path: &Path,
    connection_id: &str,
) -> anyhow::Result<GitConnectionProfile> {
    let connection_id = normalized_token(connection_id, "git_connection_id_invalid")?;
    load_git_connections(path)?
        .profiles
        .into_iter()
        .find(|profile| profile.id == connection_id)
        .ok_or_else(|| anyhow!("git_connection_not_found"))
}

pub fn upsert_git_connection(
    path: &Path,
    expected_revision: u64,
    mut profile: GitConnectionProfile,
) -> anyhow::Result<GitConnectionDocument> {
    normalize_and_validate_profile(&mut profile)?;
    let _lock = lock_git_connection_store(path)?;
    let mut document = load_git_connections(path)?;
    if document.revision != expected_revision {
        return Err(anyhow!("git_connection_revision_conflict"));
    }
    if let Some(existing) = document
        .profiles
        .iter_mut()
        .find(|candidate| candidate.id == profile.id)
    {
        *existing = profile;
    } else {
        if document.profiles.len() >= MAX_PROFILES {
            return Err(anyhow!("git_connection_limit_exceeded"));
        }
        document.profiles.push(profile);
    }
    document
        .profiles
        .sort_by(|left, right| left.id.cmp(&right.id));
    document.revision = document.revision.saturating_add(1);
    write_git_connections(path, &document)?;
    Ok(document)
}

pub fn delete_git_connection(
    path: &Path,
    expected_revision: u64,
    connection_id: &str,
) -> anyhow::Result<GitConnectionDocument> {
    let connection_id = normalized_token(connection_id, "git_connection_id_invalid")?;
    let _lock = lock_git_connection_store(path)?;
    let mut document = load_git_connections(path)?;
    if document.revision != expected_revision {
        return Err(anyhow!("git_connection_revision_conflict"));
    }
    let before = document.profiles.len();
    document
        .profiles
        .retain(|profile| profile.id != connection_id);
    if document.profiles.len() == before {
        return Err(anyhow!("git_connection_not_found"));
    }
    document.revision = document.revision.saturating_add(1);
    write_git_connections(path, &document)?;
    Ok(document)
}

pub fn normalize_and_validate_profile(profile: &mut GitConnectionProfile) -> anyhow::Result<()> {
    profile.id = normalized_token(&profile.id, "git_connection_id_invalid")?;
    profile.forge_kind = profile.forge_kind.trim().to_ascii_lowercase();
    profile.git_host = normalize_host(&profile.git_host)?;
    profile.api_host = normalize_host(&profile.api_host)?;
    profile.git_username = normalized_token(&profile.git_username, "git_username_invalid")?;
    profile.auth_scheme = profile.auth_scheme.trim().to_ascii_lowercase();
    profile.git_credential_ref = profile.git_credential_ref.trim().to_ascii_lowercase();
    profile.api_credential_ref = profile.api_credential_ref.trim().to_ascii_lowercase();
    profile.allowed_owners = normalize_allowlist(&profile.allowed_owners, "git_owner_invalid")?;
    profile.allowed_repositories =
        normalize_allowlist(&profile.allowed_repositories, "git_repository_invalid")?;

    if profile.forge_kind != "github"
        || profile.git_host != "github.com"
        || profile.api_host != "api.github.com"
        || profile.auth_scheme != "token"
        || profile.git_credential_ref != GITHUB_GIT_CREDENTIAL_REF
        || profile.api_credential_ref != GITHUB_API_CREDENTIAL_REF
    {
        return Err(anyhow!("git_connection_provider_unsupported"));
    }
    if profile.allowed_owners.is_empty() || profile.allowed_repositories.is_empty() {
        return Err(anyhow!("git_connection_allowlist_required"));
    }
    Ok(())
}

fn normalize_and_validate_document(document: &mut GitConnectionDocument) -> anyhow::Result<()> {
    if document.profiles.len() > MAX_PROFILES {
        return Err(anyhow!("git_connection_limit_exceeded"));
    }
    let mut ids = BTreeSet::new();
    for profile in &mut document.profiles {
        normalize_and_validate_profile(profile)?;
        if !ids.insert(profile.id.clone()) {
            return Err(anyhow!("git_connection_duplicate_id"));
        }
    }
    document
        .profiles
        .sort_by(|left, right| left.id.cmp(&right.id));
    Ok(())
}

fn normalize_allowlist(values: &[String], error: &'static str) -> anyhow::Result<Vec<String>> {
    if values.len() > MAX_ALLOWLIST_ITEMS {
        return Err(anyhow!(error));
    }
    let mut normalized = BTreeSet::new();
    for value in values {
        normalized.insert(normalized_repository_token(value, error)?);
    }
    Ok(normalized.into_iter().collect())
}

fn normalized_repository_token(value: &str, error: &'static str) -> anyhow::Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 100
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(anyhow!(error));
    }
    Ok(value)
}

fn normalized_token(value: &str, error: &'static str) -> anyhow::Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(anyhow!(error));
    }
    Ok(value)
}

fn normalize_host(value: &str) -> anyhow::Result<String> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 253
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(anyhow!("git_connection_host_invalid"));
    }
    Ok(value)
}

fn write_git_connections(path: &Path, document: &GitConnectionDocument) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("git_connection_store_parent_missing"))?;
    fs::create_dir_all(parent)?;
    apply_private_directory_permissions(parent)?;
    reject_symlink(path)?;
    let payload = serde_json::to_vec_pretty(document)?;
    let temp = parent.join(format!(".git-connections-{}.tmp", Uuid::new_v4().simple()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    if let Err(error) = file.write_all(&payload).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    apply_private_file_permissions(path)
}

fn lock_git_connection_store(path: &Path) -> anyhow::Result<fs::File> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("git_connection_store_parent_missing"))?;
    fs::create_dir_all(parent)?;
    apply_private_directory_permissions(parent)?;
    let lock_path = parent.join("remote-connections.lock");
    reject_symlink(&lock_path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(&lock_path)?;
    fs2::FileExt::lock_exclusive(&file)?;
    apply_private_file_permissions(&lock_path)?;
    Ok(file)
}

fn reject_symlink(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(anyhow!("git_connection_store_symlink_rejected"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn apply_private_directory_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_private_directory_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn apply_private_file_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn apply_private_file_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "git_remote_config_tests.rs"]
mod tests;
