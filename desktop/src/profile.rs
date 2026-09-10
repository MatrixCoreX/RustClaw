use crate::Result;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Connection {
    Https {
        origin: String,
        ca_pem: Option<String>,
        ca_sha256: Option<String>,
    },
    Ssh {
        host: String,
        port: u16,
        username: String,
        host_key_sha256: String,
        webd_port: u16,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: Uuid,
    pub alias: String,
    pub connection: Connection,
    #[serde(default)]
    pub saved_login: bool,
}

impl Connection {
    pub fn origin(&self) -> Result<Url> {
        match self {
            Self::Https { origin, .. } => https_origin(origin),
            Self::Ssh { webd_port, .. } => Url::parse(&format!("http://127.0.0.1:{webd_port}"))
                .map_err(|_| "address_invalid".into()),
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Https {
                origin,
                ca_pem,
                ca_sha256,
            } => {
                https_origin(origin)?;
                match (ca_pem, ca_sha256) {
                    (None, None) => {}
                    (Some(pem), Some(expected)) => {
                        if pem.len() > 32 * 1024 || pem.contains("PRIVATE KEY") {
                            return Err("public_certificate_required".into());
                        }
                        let certs = rustls_pemfile::certs(&mut Cursor::new(pem))
                            .collect::<std::result::Result<Vec<_>, _>>()
                            .map_err(|_| "certificate_invalid")?;
                        if certs.len() != 1 {
                            return Err("single_ca_certificate_required".into());
                        }
                        let digest = hex::encode(Sha256::digest(&certs[0]));
                        let expected = expected.replace(':', "").to_ascii_lowercase();
                        if digest != expected {
                            return Err("certificate_fingerprint_mismatch".into());
                        }
                    }
                    _ => return Err("certificate_fingerprint_required".into()),
                }
            }
            Self::Ssh {
                host,
                port,
                username,
                host_key_sha256,
                webd_port,
            } => {
                if *port == 0
                    || *webd_port == 0
                    || host.is_empty()
                    || username.is_empty()
                    || username.len() > 128
                    || host
                        .chars()
                        .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '#' | '?' | '\\'))
                    || !host_key_sha256.starts_with("SHA256:")
                    || host_key_sha256.len() != 50
                {
                    return Err("ssh_profile_invalid".into());
                }
            }
        }
        Ok(())
    }
}

pub fn https_origin(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| "address_invalid")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.contains('\\')
    {
        return Err("https_origin_required".into());
    }
    Ok(url)
}

/// Validate the unnormalized input before URL joining can remove dot segments.
pub fn api_path(path: &str) -> Result<()> {
    let raw_path = path.split('?').next().unwrap_or_default();
    let lower = raw_path.to_ascii_lowercase();
    if !(raw_path.starts_with("/v1/")
        || matches!(raw_path, "/webd/login" | "/webd/session" | "/webd/logout"))
        || path.contains(['\\', '#', '\r', '\n', '\0'])
        || raw_path.contains("//")
        || lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains("%25")
        || raw_path.split('/').any(|s| s == "." || s == "..")
        || path.len() > 16384
    {
        return Err("api_path_denied".into());
    }
    Ok(())
}

pub struct ProfileStore {
    directory: PathBuf,
}
impl ProfileStore {
    pub fn new(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory).map_err(|_| "profile_storage_unavailable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "profile_storage_unavailable")?;
        }
        Ok(Self { directory })
    }
    pub fn list(&self) -> Result<Vec<Profile>> {
        let path = self.directory.join("profiles-v1.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(path).map_err(|_| "profile_storage_unavailable")?;
        let profiles: Vec<Profile> =
            serde_json::from_slice(&bytes).map_err(|_| "profile_schema_invalid")?;
        for p in &profiles {
            p.connection.validate()?;
        }
        Ok(profiles)
    }
    pub fn get(&self, id: Uuid) -> Result<Profile> {
        self.list()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("profile_not_found".into())
    }
    pub fn add(&self, alias: String, connection: Connection) -> Result<Profile> {
        connection.validate()?;
        if alias.trim().is_empty() || alias.len() > 128 {
            return Err("alias_invalid".into());
        }
        let profile = Profile {
            id: Uuid::new_v4(),
            alias: alias.trim().into(),
            connection,
            saved_login: false,
        };
        let mut profiles = self.list()?;
        if profiles.len() >= 100 {
            return Err("profile_limit".into());
        }
        profiles.push(profile.clone());
        self.save(&profiles)?;
        Ok(profile)
    }
    pub fn forget(&self, id: Uuid) -> Result<()> {
        let mut profiles = self.list()?;
        profiles.retain(|p| p.id != id);
        self.save(&profiles)
    }
    pub fn mark_saved_login(&self, id: Uuid) -> Result<()> {
        let mut profiles = self.list()?;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("profile_not_found")?;
        profile.saved_login = true;
        self.save(&profiles)
    }
    fn save(&self, profiles: &[Profile]) -> Result<()> {
        use std::io::Write;
        let tmp = self
            .directory
            .join(format!("profiles-{}.tmp", Uuid::new_v4()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp).map_err(|_| "profile_storage_unavailable")?;
        file.write_all(&serde_json::to_vec_pretty(profiles).map_err(|_| "profile_schema_invalid")?)
            .map_err(|_| "profile_storage_unavailable")?;
        file.sync_all().map_err(|_| "profile_storage_unavailable")?;
        std::fs::rename(tmp, self.directory.join("profiles-v1.json"))
            .map_err(|_| "profile_storage_unavailable".into())
            .map(|_| ())
    }
}
