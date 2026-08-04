use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::InstructionSource;

pub(super) fn unloaded_instruction_source(
    path: &Path,
    logical_path: String,
    depth: usize,
    precedence: usize,
    source_layer: &'static str,
    status: &'static str,
) -> InstructionSource {
    InstructionSource {
        source_layer,
        logical_path,
        depth,
        precedence,
        source_bytes: path
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or_default(),
        loaded_bytes: 0,
        injected_bytes: 0,
        content_sha256: None,
        digest_scope: "not_loaded",
        status,
        file_budget_truncated: false,
        total_budget_truncated: false,
        content: String::new(),
    }
}

pub(super) fn read_instruction_source(
    path: &Path,
    logical_path: String,
    depth: usize,
    precedence: usize,
    source_layer: &'static str,
    max_file_bytes: usize,
) -> anyhow::Result<InstructionSource> {
    let source_bytes = path
        .metadata()
        .map_err(|error| anyhow::anyhow!("workspace_instruction_metadata_failed:{error}"))?
        .len();
    let read_limit = max_file_bytes.saturating_add(4) as u64;
    let mut bytes = Vec::with_capacity(read_limit.min(source_bytes) as usize);
    File::open(path)
        .map_err(|error| anyhow::anyhow!("workspace_instruction_open_failed:{error}"))?
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| anyhow::anyhow!("workspace_instruction_read_failed:{error}"))?;
    let file_budget_truncated = source_bytes > max_file_bytes as u64;
    let target_len = bytes.len().min(max_file_bytes);
    let valid_len = utf8_prefix_len(&bytes, target_len);
    let (content, status) = match valid_len {
        Some(valid_len) => (
            String::from_utf8(bytes[..valid_len].to_vec())
                .expect("validated workspace instruction utf-8"),
            if file_budget_truncated {
                "loaded_file_truncated"
            } else {
                "loaded"
            },
        ),
        None => (String::new(), "invalid_utf8"),
    };
    let digest_bytes = if status == "invalid_utf8" {
        bytes.as_slice()
    } else {
        content.as_bytes()
    };
    let digest_scope = if file_budget_truncated {
        "loaded_prefix"
    } else {
        "full_source"
    };
    Ok(InstructionSource {
        source_layer,
        logical_path,
        depth,
        precedence,
        source_bytes,
        loaded_bytes: content.len(),
        injected_bytes: 0,
        content_sha256: Some(hex::encode(Sha256::digest(digest_bytes))),
        digest_scope,
        status,
        file_budget_truncated,
        total_budget_truncated: false,
        content,
    })
}

pub(super) fn utf8_prefix_len(bytes: &[u8], target_len: usize) -> Option<usize> {
    let validation_len = bytes.len().min(target_len.saturating_add(4));
    match std::str::from_utf8(&bytes[..validation_len]) {
        Ok(_) => Some(target_len),
        Err(error) if error.error_len().is_none() && error.valid_up_to() <= target_len => {
            Some(error.valid_up_to())
        }
        Err(error) if error.valid_up_to() >= target_len => Some(target_len),
        Err(_) => None,
    }
    .and_then(|mut end| {
        while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
            end -= 1;
        }
        std::str::from_utf8(&bytes[..end]).ok().map(|_| end)
    })
}
