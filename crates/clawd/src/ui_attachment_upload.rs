//! An immutable client-message batch. Validate the entire batch before any file is published.
use super::*;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::io::{self, Read, Seek, SeekFrom};

pub(crate) struct UploadInput<R = io::Cursor<Vec<u8>>> {
    pub name: String,
    pub mime_type: String,
    pub kind: String,
    pub bytes: R,
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

pub(crate) fn commit_upload_batch<R: Read + Seek>(
    root: &Path,
    user_id: i64,
    chat_id: i64,
    call_id: &str,
    inputs: Vec<UploadInput<R>>,
) -> Result<Vec<MaterializedAttachment>, String> {
    if inputs.is_empty() || inputs.len() > MAX_UI_ATTACHMENTS {
        return Err("ui_attachments_too_many".into());
    }
    let mut total = 0usize;
    let scope = hex::encode(Sha256::digest(call_id.as_bytes()));
    let directory = root.join(format!("data/ui/{user_id}/{chat_id}/{scope}"));
    let mut prepared = Vec::new();
    for (index, mut input) in inputs.into_iter().enumerate() {
        let size = input
            .bytes
            .seek(SeekFrom::End(0))
            .map_err(|_| "ui_attachment_stream_failed")?;
        if size > MAX_UI_ATTACHMENT_BYTES as u64 {
            return Err("ui_attachment_too_large".into());
        }
        total = total.saturating_add(size as usize);
        if total > MAX_UI_TOTAL_ATTACHMENT_BYTES {
            return Err("ui_attachments_total_too_large".into());
        }
        let name = safe_upload_filename(&input.name, &input.kind, &input.mime_type);
        input
            .bytes
            .rewind()
            .map_err(|_| "ui_attachment_stream_failed")?;
        let mut hasher = Sha256::new();
        io::copy(&mut input.bytes, &mut hasher).map_err(|_| "ui_attachment_stream_failed")?;
        let digest = hex::encode(hasher.finalize());
        input
            .bytes
            .rewind()
            .map_err(|_| "ui_attachment_stream_failed")?;
        let relative = format!("data/ui/{user_id}/{chat_id}/{scope}/{index:02}-{digest}-{name}");
        let metadata = MaterializedAttachment {
            name,
            mime_type: input.mime_type,
            kind: normalize_kind(&input.kind, ""),
            rel_path: relative,
            size: size as usize,
        };
        prepared.push((metadata, input.bytes, digest));
    }
    let manifest =
        serde_json::to_vec(&prepared.iter().map(|(item, _, _)| item).collect::<Vec<_>>())
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
            for (item, _, expected_digest) in &prepared {
                let mut hasher = Sha256::new();
                let mut file = crate::secure_workspace_fs::open_workspace_file(
                    root,
                    &root.join(&item.rel_path),
                )
                .map_err(|_| "ui_attachment_replay_missing")?
                .take((MAX_UI_ATTACHMENT_BYTES + 1) as u64);
                let length = io::copy(&mut file, &mut hasher)
                    .map_err(|_| "ui_attachment_replay_unreadable")?;
                if length != item.size as u64 || hex::encode(hasher.finalize()) != *expected_digest
                {
                    return Err("ui_attachment_replay_digest_mismatch".into());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs2::available_space(root).unwrap_or(0) < 2 * 1024 * 1024 * 1024 + total as u64 {
                return Err("ui_attachment_storage_pressure".into());
            }
            let mut rollback = Rollback {
                root,
                created: Vec::new(),
                committed: false,
            };
            for (item, bytes, _) in &mut prepared {
                let target = root.join(&item.rel_path);
                crate::secure_workspace_fs::atomic_create_workspace_stream(root, &target, bytes)
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
    Ok(prepared
        .into_iter()
        .map(|(metadata, _, _)| metadata)
        .collect())
}

#[cfg(test)]
#[path = "ui_attachment_upload_tests.rs"]
mod tests;

pub(crate) fn validate_upload_references(
    root: &Path,
    user_id: i64,
    chat_id: i64,
    call_id: &str,
    payload: &mut Value,
) -> Result<(), String> {
    let Some(items) = payload
        .get("attachments")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    else {
        if ["images", "audio", "audios", "video", "videos", "files"]
            .iter()
            .filter_map(|key| payload.get(*key))
            .any(|value| !value.is_null() && value.as_array().is_none_or(|items| !items.is_empty()))
        {
            return Err("ui_attachment_reference_missing".into());
        }
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
    // Media aliases are projections of the verified manifest, never independent locators.
    rewrite_payload_attachments(payload, &manifest);
    Ok(())
}
