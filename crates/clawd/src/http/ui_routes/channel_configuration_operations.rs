#[derive(Debug, thiserror::Error)]
enum ChannelConfigurationOperationError {
    #[error("channel_configuration_unavailable")]
    Unavailable,
    #[error("channel_configuration_mismatch")]
    Mismatch,
    #[error("channel_configuration_operation_poisoned")]
    Poisoned,
}

type ChannelConfigurationLockKey = (PathBuf, String);
type ChannelConfigurationLock = tokio::sync::Mutex<()>;

fn official_registration_service_platform(service: &str) -> Option<AgentAppChannel> {
    match service {
        "feishud" => Some(AgentAppChannel::Feishu),
        "larkd" => Some(AgentAppChannel::Lark),
        _ => None,
    }
}

fn channel_configuration_slot(
    state: &AppState,
    platform: AgentAppChannel,
) -> anyhow::Result<Arc<ChannelConfigurationLock>> {
    static SLOTS: OnceLock<
        Mutex<HashMap<ChannelConfigurationLockKey, std::sync::Weak<ChannelConfigurationLock>>>,
    > = OnceLock::new();
    let root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .map_err(|_| ChannelConfigurationOperationError::Unavailable)?;
    let mut slots = SLOTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| ChannelConfigurationOperationError::Poisoned)?;
    slots.retain(|_, slot| slot.strong_count() > 0);
    let key = (root, platform.channel().to_string());
    if let Some(slot) = slots.get(&key).and_then(std::sync::Weak::upgrade) {
        return Ok(slot);
    }
    let slot = Arc::new(ChannelConfigurationLock::new(()));
    slots.insert(key, Arc::downgrade(&slot));
    Ok(slot)
}

async fn lock_channel_configuration(
    state: &AppState,
    platform: AgentAppChannel,
) -> anyhow::Result<tokio::sync::OwnedMutexGuard<()>> {
    Ok(channel_configuration_slot(state, platform)?
        .lock_owned()
        .await)
}

async fn begin_channel_configuration_mutation(
    state: &AppState,
    platform: AgentAppChannel,
    actor_key: &str,
) -> anyhow::Result<tokio::sync::OwnedMutexGuard<()>> {
    let guard = lock_channel_configuration(state, platform).await?;
    require_current_registration_admin(state, actor_key)?;
    {
        let mut db = state.core.db.get()?;
        crate::repo::channel_registration::advance_configuration_revision(
            &mut db,
            platform.channel(),
        )?;
    }
    Ok(guard)
}

fn channel_configuration_paths(state: &AppState, platform: AgentAppChannel) -> [PathBuf; 2] {
    let file = format!("{}.toml", platform.channel());
    [
        state
            .skill_rt
            .workspace_root
            .join("configs/channels")
            .join(&file),
        state
            .skill_rt
            .workspace_root
            .join("docker/config/channels")
            .join(file),
    ]
}

fn read_registration_configuration(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    const MAX_BYTES: u64 = 1024 * 1024;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Preserve configured symlinks, but never wait for a FIFO writer.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ChannelConfigurationOperationError::Unavailable.into()),
    };
    let metadata = file
        .metadata()
        .map_err(|_| ChannelConfigurationOperationError::Unavailable)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(ChannelConfigurationOperationError::Unavailable.into());
    }
    let mut raw = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|_| ChannelConfigurationOperationError::Unavailable)?;
    if raw.len() as u64 > MAX_BYTES {
        return Err(ChannelConfigurationOperationError::Unavailable.into());
    }
    Ok(Some(raw))
}

fn channel_configuration_fingerprint(
    state: &AppState,
    platform: AgentAppChannel,
) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut copies = Vec::new();
    for path in channel_configuration_paths(state, platform) {
        copies.push(
            read_registration_configuration(&path)?
                .map(|raw| format!("{:x}", Sha256::digest(&raw))),
        );
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&copies)?)
    ))
}

fn registration_credential_fields_match(
    raw: &[u8],
    platform: AgentAppChannel,
    app_id: &str,
    app_secret: &str,
    require_enabled: bool,
) -> bool {
    let app_id = app_id.trim();
    let app_secret = app_secret.trim();
    let Some(value) = std::str::from_utf8(raw)
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(raw).ok())
    else {
        return false;
    };
    let Some(channel) = value.get(platform.channel()) else {
        return false;
    };
    !app_id.is_empty()
        && !app_secret.is_empty()
        && channel
            .get("app_id")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            == Some(app_id)
        && channel
            .get("app_secret")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            == Some(app_secret)
        && (!require_enabled || channel.get("enabled").and_then(toml::Value::as_bool) == Some(true))
}

fn channel_configuration_copies_match(
    state: &AppState,
    platform: AgentAppChannel,
    app_id: &str,
    app_secret: &str,
) -> bool {
    channel_configuration_paths(state, platform)
        .iter()
        .all(|path| {
            read_registration_configuration(path)
                .ok()
                .flatten()
                .is_some_and(|raw| {
                    registration_credential_fields_match(&raw, platform, app_id, app_secret, false)
                })
        })
}

fn verify_applied_channel_configuration(
    state: &AppState,
    platform: AgentAppChannel,
    app_id: &str,
    app_secret: &str,
) -> anyhow::Result<()> {
    let paths = channel_configuration_paths(state, platform);
    let first = read_registration_configuration(&paths[0])?
        .ok_or(ChannelConfigurationOperationError::Mismatch)?;
    let second = read_registration_configuration(&paths[1])?
        .ok_or(ChannelConfigurationOperationError::Mismatch)?;
    if first != second
        || !registration_credential_fields_match(&first, platform, app_id, app_secret, true)
    {
        return Err(ChannelConfigurationOperationError::Mismatch.into());
    }
    Ok(())
}

fn channel_configuration_is_ready(
    state: &AppState,
    platform: AgentAppChannel,
    app_id: &str,
    app_secret: &str,
) -> bool {
    channel_configuration_copies_match(state, platform, app_id, app_secret)
        && (|| -> anyhow::Result<bool> {
            let db = state.core.db.get()?;
            Ok(
                !crate::repo::channel_registration::configuration_requires_recovery(
                    &db,
                    platform.channel(),
                )?,
            )
        })()
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "channel_configuration_operation_tests.rs"]
mod channel_configuration_operation_tests;
