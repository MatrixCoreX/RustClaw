use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use claw_core::conversation_input::ConversationInputClientTaskRequest;
use sha2::{Digest, Sha256};

use crate::repo::conversation_inputs::ConversationInputAttachmentBinding;

pub(super) fn prepare_attachment_bindings(
    workspace_root: &Path,
    owner_principal_id: &str,
    request: &ConversationInputClientTaskRequest,
) -> Result<Vec<ConversationInputAttachmentBinding>, &'static str> {
    let Some(ingress) = request.task.ingress.as_ref() else {
        return Ok(Vec::new());
    };
    if ingress.attachments.is_empty() {
        return Ok(Vec::new());
    }
    crate::ui_attachments::validate_channel_ingress_attachments(
        workspace_root,
        &ingress.attachments,
    )?;
    ingress
        .attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            let path = workspace_root.join(&attachment.path);
            let (sha256, size_bytes) = hash_file(&path)?;
            let mut identity = Sha256::new();
            identity.update(owner_principal_id.as_bytes());
            identity.update([0]);
            identity.update(request.input.scope.agent_id.as_bytes());
            identity.update([0]);
            identity.update(request.input.scope.channel.as_bytes());
            identity.update([0]);
            identity.update(request.input.scope.channel_account_id.as_bytes());
            identity.update([0]);
            identity.update(request.input.scope.conversation_id.as_bytes());
            identity.update([0]);
            identity.update(request.input.client_message_id.as_bytes());
            identity.update([0]);
            identity.update(index.to_string().as_bytes());
            identity.update([0]);
            identity.update(sha256.as_bytes());
            let identity = format!("channel_attachment:{:x}", identity.finalize());
            Ok(ConversationInputAttachmentBinding {
                attachment_id: identity,
                kind: attachment.kind.trim().to_string(),
                workspace_rel_path: attachment.path.trim().to_string(),
                mime_type: attachment.mime_type.clone(),
                display_name: display_name(&attachment.path),
                size_bytes,
                sha256,
            })
        })
        .collect()
}

fn hash_file(path: &Path) -> Result<(String, u64), &'static str> {
    let mut file = File::open(path).map_err(|_| "channel_attachment_missing")?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "channel_attachment_read_failed")?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), size))
}

fn display_name(path: &str) -> Option<String> {
    PathBuf::from(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
