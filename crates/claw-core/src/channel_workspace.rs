//! Put transport-owned inbound files into the authenticated account's workspace.
//! Only the adapter which received the bytes stages them. The task HTTP boundary
//! accepts references inside the caller's workspace and never reads a host inbox.
use crate::secure_workspace_fs as fs;
use crate::types::{ApiResponse, AuthIdentity, SubmitTaskRequest};
use anyhow::{ensure, Context};
use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

pub const WORKSPACES: &str = "data/principal-workspaces";
const FILE_LIMIT: u64 = 100 * 1024 * 1024;
const BATCH_LIMIT: u64 = 250 * 1024 * 1024;

fn safe_telegram_storage_segment(raw: &str, fallback: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.trim_matches('_').is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

pub fn telegram_inbox_path(
    root_dir: &str,
    bot_name: &str,
    chat_id: i64,
    user_id: i64,
    provider_message_id: &str,
    ext: &str,
) -> String {
    let base = root_dir.trim().trim_end_matches('/');
    let safe_bot = safe_telegram_storage_segment(bot_name, "bot");
    let safe_message_id = safe_telegram_storage_segment(provider_message_id, "message");
    let safe_ext = ext.trim().trim_start_matches('.');
    if base.is_empty() {
        format!("{safe_bot}/{chat_id}/{user_id}/{safe_message_id}.{safe_ext}")
    } else {
        format!("{base}/{safe_bot}/{chat_id}/{user_id}/{safe_message_id}.{safe_ext}")
    }
}

pub fn private_directory(runtime: &Path, principal: &str) -> anyhow::Result<PathBuf> {
    ensure!(!principal.trim().is_empty(), "workspace_principal_missing");
    let namespace = runtime.join(WORKSPACES);
    let path = namespace.join(format!("{:x}", Sha256::digest(principal.as_bytes())));
    fs::ensure_private_workspace_directory(runtime, &namespace)?;
    fs::ensure_private_workspace_directory(&namespace, &path)?;
    Ok(path)
}

pub async fn stage_received_attachments(
    client: &reqwest::Client,
    core_url: &str,
    request: &SubmitTaskRequest,
) -> anyhow::Result<()> {
    let Some(ingress) = request
        .ingress
        .as_ref()
        .filter(|i| !i.attachments.is_empty())
    else {
        return Ok(());
    };
    let key = request
        .user_key
        .as_deref()
        .context("channel_attachment_identity_required")?;
    let reply: ApiResponse<AuthIdentity> = client
        .get(format!("{}/v1/auth/me", core_url.trim_end_matches('/')))
        .header(crate::product_identity::AUTH_KEY_HEADER, key)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let actor = reply
        .data
        .filter(|actor| reply.ok && actor.user_key == key)
        .context("channel_attachment_identity_unavailable")?;
    if actor.role.eq_ignore_ascii_case("admin") {
        return Ok(());
    }
    let root = std::env::current_dir()?;
    let attachments = ingress.attachments.clone();
    tokio::task::spawn_blocking(move || {
        let own = private_directory(&root, &actor.principal_id)?;
        stage_inbox_files(&root, &own, &attachments)
    })
    .await?
}

pub fn stage_inbox_files(
    root: &Path,
    own: &Path,
    attachments: &[crate::channel_ingress::ChannelIngressAttachment],
) -> anyhow::Result<()> {
    ensure!(attachments.len() <= 20, "channel_attachments_too_many");
    let mut total = 0;
    for item in attachments {
        let relative = Path::new(&item.path);
        ensure!(
            !relative.as_os_str().is_empty()
                && relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "channel_attachment_path_invalid"
        );
        let mut input = fs::open_workspace_file(root, &root.join(relative))?;
        let size = input.metadata()?.len();
        total += size.min(FILE_LIMIT + 1);
        ensure!(
            size <= FILE_LIMIT && total <= BATCH_LIMIT && item.size.is_none_or(|n| n == size),
            "channel_attachment_size_invalid"
        );
        let target = own.join(relative);
        // Publish only complete files. The no-replace operation also handles
        // concurrent retries that finish staging the same path.
        match fs::create_workspace_stream_with(own, &target, |output| {
            if io::copy(&mut (&mut input).take(size + 1), output)? != size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "channel_attachment_size_changed",
                ));
            }
            Ok(())
        }) {
            Ok(()) => continue,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        use std::io::Seek;
        input.rewind()?;
        let existing = fs::open_workspace_file(own, &target)?;
        ensure!(
            existing.metadata()?.len() == size && input.metadata()?.len() == size,
            "channel_attachment_size_changed"
        );
        let mut expected = Sha256::new();
        let mut actual = Sha256::new();
        io::copy(&mut input.take(size + 1), &mut expected)?;
        io::copy(&mut existing.take(size + 1), &mut actual)?;
        ensure!(
            expected.finalize() == actual.finalize(),
            "channel_attachment_content_changed"
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "channel_workspace_tests.rs"]
mod tests;
