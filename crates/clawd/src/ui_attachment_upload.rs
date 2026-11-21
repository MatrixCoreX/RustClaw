//! An immutable client-message batch. Validate the entire batch before any file is published.
use super::*;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::io::Read;

pub(crate) struct UploadInput {
    pub name: String,
    pub mime_type: String,
    pub kind: String,
    pub bytes: Vec<u8>,
}

struct Rollback<'a> {
    root: &'a Path,
    created: Vec<std::path::PathBuf>,
    committed: bool,
}
impl Drop for Rollback<'_> {
    fn drop(&mut self) {
        if !self.committed {
            for path in self.created.iter().rev() {
                let _ = crate::secure_workspace_fs::remove_workspace_file(self.root, path);
            }
        }
    }
}

pub(crate) fn commit_upload_batch(
    root: &Path,
    user_id: i64,
    chat_id: i64,
    call_id: &str,
    inputs: Vec<UploadInput>,
) -> Result<Vec<MaterializedAttachment>, String> {
    if inputs.is_empty() || inputs.len() > MAX_UI_ATTACHMENTS {
        return Err("ui_attachments_too_many".into());
    }
    let mut total = 0usize;
    let scope = hex::encode(Sha256::digest(call_id.as_bytes()));
    let directory = root.join(format!("data/ui/{user_id}/{chat_id}/{scope}"));
    let mut prepared = Vec::new();
    for (index, input) in inputs.into_iter().enumerate() {
        if input.bytes.len() > MAX_UI_ATTACHMENT_BYTES {
            return Err("ui_attachment_too_large".into());
        }
        total = total.saturating_add(input.bytes.len());
        if total > MAX_UI_TOTAL_ATTACHMENT_BYTES {
            return Err("ui_attachments_total_too_large".into());
        }
        let name = safe_upload_filename(&input.name, &input.kind, &input.mime_type);
        let digest = hex::encode(Sha256::digest(&input.bytes));
        let relative = format!("data/ui/{user_id}/{chat_id}/{scope}/{index:02}-{digest}-{name}");
        let metadata = MaterializedAttachment {
            name,
            mime_type: input.mime_type,
            kind: normalize_kind(&input.kind, ""),
            rel_path: relative,
            size: input.bytes.len(),
        };
        prepared.push((metadata, input.bytes));
    }
    let manifest = serde_json::to_vec(&prepared.iter().map(|(item, _)| item).collect::<Vec<_>>())
        .map_err(|_| "ui_attachment_manifest_invalid")?;
    crate::secure_workspace_fs::ensure_private_workspace_directory(root, &directory)
        .map_err(|_| "ui_attachment_directory_unavailable")?;
    let lock_path = directory.join(".upload-lock");
    match crate::secure_workspace_fs::atomic_create_workspace_file(root, &lock_path, b"") {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("ui_attachment_lock_unavailable".into()),
    }
    let lock = crate::secure_workspace_fs::open_workspace_file(root, &lock_path)
        .map_err(|_| "ui_attachment_lock_unavailable")?;
    lock.try_lock_exclusive()
        .map_err(|_| "ui_attachment_upload_busy")?;
    let manifest_path = directory.join(".upload-manifest.json");
    match crate::secure_workspace_fs::open_workspace_file(root, &manifest_path) {
        Ok(file) => {
            let mut existing = Vec::new();
            file.take(64 * 1024)
                .read_to_end(&mut existing)
                .map_err(|_| "ui_attachment_manifest_invalid")?;
            if existing != manifest {
                return Err("ui_attachment_replay_conflict".into());
            }
            // Replays verify immutable file contents, never overwrite them.
            for (item, expected) in &prepared {
                let mut actual = Vec::new();
                crate::secure_workspace_fs::open_workspace_file(root, &root.join(&item.rel_path))
                    .map_err(|_| "ui_attachment_replay_missing")?
                    .take((MAX_UI_ATTACHMENT_BYTES + 1) as u64)
                    .read_to_end(&mut actual)
                    .map_err(|_| "ui_attachment_replay_unreadable")?;
                if actual != *expected {
                    return Err("ui_attachment_replay_digest_mismatch".into());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut rollback = Rollback {
                root,
                created: Vec::new(),
                committed: false,
            };
            for (item, bytes) in &prepared {
                let target = root.join(&item.rel_path);
                crate::secure_workspace_fs::atomic_create_workspace_file(root, &target, bytes)
                    .map_err(|_| "ui_attachment_immutable_write_failed")?;
                rollback.created.push(target);
            }
            crate::secure_workspace_fs::atomic_create_workspace_file(
                root,
                &manifest_path,
                &manifest,
            )
            .map_err(|_| "ui_attachment_manifest_commit_failed")?;
            rollback.committed = true;
        }
        Err(_) => return Err("ui_attachment_manifest_unavailable".into()),
    }
    Ok(prepared.into_iter().map(|(metadata, _)| metadata).collect())
}

#[cfg(test)]
#[path = "ui_attachment_upload_tests.rs"]
mod tests;

pub(crate) fn validate_upload_references(
    root: &Path,
    user_id: i64,
    chat_id: i64,
    call_id: &str,
    payload: &Value,
) -> Result<(), String> {
    let Some(items) = payload
        .get("attachments")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    else {
        return Ok(());
    };
    let expected = format!(
        "data/ui/{user_id}/{chat_id}/{}",
        hex::encode(Sha256::digest(call_id.as_bytes()))
    );
    if items.iter().any(|item| {
        !item
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.starts_with(&format!("{expected}/")))
    }) {
        return Err("ui_attachment_reference_owner_mismatch".into());
    }
    let file = crate::secure_workspace_fs::open_workspace_file(
        root,
        &root.join(&expected).join(".upload-manifest.json"),
    )
    .map_err(|_| "ui_attachment_manifest_unavailable")?;
    let mut bytes = Vec::new();
    file.take(64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "ui_attachment_manifest_invalid")?;
    let manifest: Vec<MaterializedAttachment> =
        serde_json::from_slice(&bytes).map_err(|_| "ui_attachment_manifest_invalid")?;
    if items.len() != manifest.len() {
        return Err("ui_attachment_reference_mismatch".into());
    }
    for (item, stored) in items.iter().zip(&manifest) {
        if item.get("path").and_then(Value::as_str) != Some(stored.rel_path.as_str())
            || item.get("kind").and_then(Value::as_str) != Some(stored.kind.as_str())
            || item.get("mime_type").and_then(Value::as_str) != Some(stored.mime_type.as_str())
            || item.get("size").and_then(Value::as_u64) != Some(stored.size as u64)
        {
            return Err("ui_attachment_reference_mismatch".into());
        }
    }
    Ok(())
}
