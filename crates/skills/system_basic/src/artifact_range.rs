use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use super::*;

const DEFAULT_ARTIFACT_PAGE_BYTES: u64 = 64 * 1024;
const MAX_ARTIFACT_PAGE_BYTES: u64 = 1024 * 1024;

pub(super) fn read_artifact_range(
    workspace_root: &Path,
    obj: &Map<String, Value>,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let resolved = resolve_path(workspace_root, path, false)?;
    let artifact_root = workspace_root.join(".rustclaw").join("artifacts");
    let canonical_artifact_root = artifact_root
        .canonicalize()
        .map_err(|err| SkillError::io("resolve_artifact_root", &artifact_root, err))?;
    if !resolved.starts_with(&canonical_artifact_root) {
        return Err(SkillError::path_denied(
            "artifact path is outside the workspace artifact root",
        ));
    }
    let metadata =
        std::fs::metadata(&resolved).map_err(|err| SkillError::io("metadata", &resolved, err))?;
    if !metadata.is_file() {
        return Err(SkillError::invalid_input(
            "artifact range target must be a regular file",
        ));
    }

    let size_bytes = metadata.len();
    let requested_start = obj
        .get("start_byte")
        .or_else(|| obj.get("cursor"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let start_byte = requested_start.min(size_bytes);
    let max_bytes = obj
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_ARTIFACT_PAGE_BYTES)
        .clamp(256, MAX_ARTIFACT_PAGE_BYTES);
    let requested_end = start_byte.saturating_add(max_bytes).min(size_bytes);
    let read_len = requested_end.saturating_sub(start_byte) as usize;

    let mut file =
        File::open(&resolved).map_err(|err| SkillError::io("open_artifact", &resolved, err))?;
    file.seek(SeekFrom::Start(start_byte))
        .map_err(|err| SkillError::io("seek_artifact", &resolved, err))?;
    let mut bytes = vec![0_u8; read_len];
    file.read_exact(&mut bytes)
        .map_err(|err| SkillError::io("read_artifact", &resolved, err))?;

    let (encoding, content, emitted_bytes) = match utf8_page(&bytes) {
        Some((text, valid_bytes)) => ("utf-8", text.to_string(), valid_bytes),
        None => ("base64", BASE64_STANDARD.encode(&bytes), bytes.len()),
    };
    let end_byte = start_byte.saturating_add(emitted_bytes as u64);
    let has_more = end_byte < size_bytes;
    let previous_start_byte = (start_byte > 0).then_some(start_byte.saturating_sub(max_bytes));
    let sha256 = sha256_file(&resolved)?;

    Ok(json!({
        "action": "read_artifact_range",
        "path": path,
        "resolved_path": resolved.display().to_string(),
        "artifact_root": canonical_artifact_root.display().to_string(),
        "content": content,
        "encoding": encoding,
        "binary": encoding == "base64",
        "size_bytes": size_bytes,
        "returned_bytes": emitted_bytes,
        "sha256": sha256,
        "content_hash": format!("sha256:{sha256}"),
        "truncated": start_byte > 0 || has_more,
        "page": {
            "cursor": start_byte,
            "start_byte": start_byte,
            "end_byte": end_byte,
            "total_bytes": size_bytes,
            "limit_bytes": max_bytes,
            "has_more": has_more,
            "next_cursor": has_more.then_some(end_byte),
            "next_start_byte": has_more.then_some(end_byte),
            "previous_cursor": previous_start_byte,
            "previous_start_byte": previous_start_byte,
        },
    })
    .to_string())
}

fn utf8_page(bytes: &[u8]) -> Option<(&str, usize)> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some((text, bytes.len()));
    }
    for trim in 1..=3.min(bytes.len()) {
        let candidate = &bytes[..bytes.len() - trim];
        if let Ok(text) = std::str::from_utf8(candidate) {
            return Some((text, candidate.len()));
        }
    }
    None
}

fn sha256_file(path: &Path) -> SkillResult<String> {
    let mut file = File::open(path).map_err(|err| SkillError::io("open_artifact", path, err))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| SkillError::io("hash_artifact", path, err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
#[path = "artifact_range_tests.rs"]
mod tests;
