use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::SessionEntry;

const SESSION_STORE_SCHEMA_VERSION: u32 = 1;
const MAX_SESSION_STORE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SESSION_COUNT: usize = 10_000;

#[derive(Deserialize, Serialize)]
struct SessionStoreDocument {
    schema_version: u32,
    sessions: HashMap<String, SessionEntry>,
}

pub(super) fn load_sessions(
    path: &Path,
    now_unix: u64,
) -> anyhow::Result<HashMap<String, SessionEntry>> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(HashMap::new());
    }
    let metadata = fs::metadata(path).with_context(|| {
        format!(
            "webd_session_store_metadata_read_failed:path={}",
            path.display()
        )
    })?;
    if metadata.len() > MAX_SESSION_STORE_BYTES {
        anyhow::bail!("webd_session_store_too_large");
    }
    let raw = fs::read(path)
        .with_context(|| format!("webd_session_store_read_failed:path={}", path.display()))?;
    let mut document: SessionStoreDocument =
        serde_json::from_slice(&raw).context("webd_session_store_parse_failed")?;
    if document.schema_version != SESSION_STORE_SCHEMA_VERSION {
        anyhow::bail!("webd_session_store_schema_unsupported");
    }
    document.sessions.retain(|session_id, entry| {
        uuid::Uuid::parse_str(session_id).is_ok()
            && !entry.user_key.trim().is_empty()
            && entry.user_key.len() <= 1024
            && entry.expires_unix > now_unix
    });
    if document.sessions.len() > MAX_SESSION_COUNT {
        anyhow::bail!("webd_session_store_entry_limit_exceeded");
    }
    Ok(document.sessions)
}

pub(super) fn persist_sessions(
    path: &Path,
    sessions: &HashMap<String, SessionEntry>,
) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    if sessions.len() > MAX_SESSION_COUNT {
        anyhow::bail!("webd_session_store_entry_limit_exceeded");
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "webd_session_store_create_dir_failed:path={}",
                parent.display()
            )
        })?;
    }
    let document = SessionStoreDocument {
        schema_version: SESSION_STORE_SCHEMA_VERSION,
        sessions: sessions.clone(),
    };
    let encoded = serde_json::to_vec(&document).context("webd_session_store_encode_failed")?;
    if encoded.len() as u64 > MAX_SESSION_STORE_BYTES {
        anyhow::bail!("webd_session_store_too_large");
    }
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).with_context(|| {
        format!(
            "webd_session_store_temporary_open_failed:path={}",
            temporary.display()
        )
    })?;
    file.write_all(&encoded)
        .context("webd_session_store_temporary_write_failed")?;
    file.sync_all()
        .context("webd_session_store_temporary_sync_failed")?;
    drop(file);
    fs::rename(&temporary, path)
        .with_context(|| format!("webd_session_store_replace_failed:path={}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .context("webd_session_store_permissions_failed")?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_store_tests.rs"]
mod tests;
