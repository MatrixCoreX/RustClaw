use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::chat_session::{SessionAttachmentRef, WorkingDirectoryIdentity};

pub(crate) const MAX_ATTACHMENTS: usize = 10;
pub(crate) const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;
pub(crate) const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 60 * 1024 * 1024;
const DIRECT_TEXT_MATERIALIZATION_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestedAttachmentKind {
    File,
    Image,
}

pub(crate) fn inspect_attachment(
    workspace: &WorkingDirectoryIdentity,
    input: &Path,
    requested_kind: RequestedAttachmentKind,
) -> Result<SessionAttachmentRef> {
    let workspace_root = Path::new(&workspace.canonical_path);
    let joined = if input.is_absolute() {
        input.to_path_buf()
    } else {
        workspace_root.join(input)
    };
    reject_symlink_components(workspace_root, &joined)?;
    let canonical = joined.canonicalize().with_context(|| {
        format!(
            "chat_attachment_path_unavailable:{path}",
            path = input.display()
        )
    })?;
    if !canonical.starts_with(workspace_root) {
        anyhow::bail!("chat_attachment_path_outside_workspace");
    }
    let metadata = fs::metadata(&canonical).context("chat_attachment_metadata_failed")?;
    if !metadata.is_file() {
        anyhow::bail!("chat_attachment_not_regular_file");
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        anyhow::bail!("chat_attachment_too_large");
    }
    reject_sensitive_path(&canonical)?;
    let bytes = fs::read(&canonical).context("chat_attachment_read_failed")?;
    let detected = detect_content(&bytes, &canonical);
    if requested_kind == RequestedAttachmentKind::Image && detected.kind != "image" {
        anyhow::bail!("chat_attachment_image_type_required");
    }
    let relative = canonical
        .strip_prefix(workspace_root)
        .map_err(|_| anyhow::anyhow!("chat_attachment_path_outside_workspace"))?;
    let materialization = if detected.kind == "file"
        && detected.mime_type.starts_with("text/")
        && metadata.len() <= DIRECT_TEXT_MATERIALIZATION_BYTES
    {
        "bounded_text_context"
    } else {
        "server_artifact_ref"
    };
    Ok(SessionAttachmentRef {
        canonical_path: canonical.to_string_lossy().into_owned(),
        display_path: relative.to_string_lossy().into_owned(),
        kind: detected.kind.to_string(),
        mime_type: detected.mime_type,
        size: metadata.len(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        materialization: materialization.to_string(),
        truncated: false,
    })
}

pub(crate) fn merge_attachment(
    attachments: &mut Vec<SessionAttachmentRef>,
    attachment: SessionAttachmentRef,
) -> Result<()> {
    if attachments
        .iter()
        .any(|existing| existing.sha256 == attachment.sha256)
    {
        return Ok(());
    }
    if attachments.len() >= MAX_ATTACHMENTS {
        anyhow::bail!("chat_attachments_too_many");
    }
    let total = attachments
        .iter()
        .map(|item| item.size)
        .sum::<u64>()
        .saturating_add(attachment.size);
    if total > MAX_TOTAL_ATTACHMENT_BYTES {
        anyhow::bail!("chat_attachments_total_too_large");
    }
    attachments.push(attachment);
    Ok(())
}

pub(crate) fn attachment_payload(attachments: &[SessionAttachmentRef]) -> Result<Vec<Value>> {
    attachments
        .iter()
        .map(|attachment| {
            let bytes =
                fs::read(&attachment.canonical_path).context("chat_attachment_read_failed")?;
            let current_sha256 = format!("{:x}", Sha256::digest(&bytes));
            if current_sha256 != attachment.sha256 {
                anyhow::bail!("chat_attachment_precondition_failed");
            }
            let name = Path::new(&attachment.canonical_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("attachment.bin");
            Ok(json!({
                "name": name,
                "mime_type": attachment.mime_type,
                "kind": attachment.kind,
                "base64": format!(
                    "data:{};base64,{}",
                    attachment.mime_type,
                    BASE64_STANDARD.encode(bytes)
                ),
                "sha256": attachment.sha256,
                "source_ref": attachment.display_path,
            }))
        })
        .collect()
}

pub(crate) fn extract_path_references(input: &str) -> Result<Vec<PathBuf>> {
    let chars = input.char_indices().collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut index = 0usize;
    let mut quote = None;
    let mut inline_code = false;
    let mut fenced_code = false;
    while index < chars.len() {
        let (byte_index, ch) = chars[index];
        if ch == '`' && quote.is_none() {
            let run = backtick_run(&chars, index);
            if run >= 3 {
                fenced_code = !fenced_code;
            } else if !fenced_code {
                inline_code = !inline_code;
            }
            index += run;
            continue;
        }
        if fenced_code || inline_code {
            index += 1;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = match quote {
                Some(active) if active == ch => None,
                None => Some(ch),
                other => other,
            };
            index += 1;
            continue;
        }
        if ch != '@' || quote.is_some() || !reference_boundary(input, byte_index) {
            index += 1;
            continue;
        }
        let (path, consumed) = parse_reference_path(input, &chars, index + 1)?;
        if !path.as_os_str().is_empty() {
            paths.push(path);
        }
        index = consumed.max(index + 1);
    }
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    Ok(paths)
}

fn parse_reference_path(
    input: &str,
    chars: &[(usize, char)],
    start: usize,
) -> Result<(PathBuf, usize)> {
    let Some((_, first)) = chars.get(start).copied() else {
        return Ok((PathBuf::new(), start));
    };
    if matches!(first, '\'' | '"') {
        let quote = first;
        let mut escaped = false;
        let mut value = String::new();
        let mut index = start + 1;
        while index < chars.len() {
            let ch = chars[index].1;
            if escaped {
                value.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                return Ok((PathBuf::from(value), index + 1));
            } else {
                value.push(ch);
            }
            index += 1;
        }
        anyhow::bail!("chat_path_reference_unterminated_quote");
    }
    let start_byte = chars[start].0;
    let mut end = start;
    while end < chars.len() {
        let ch = chars[end].1;
        if ch.is_whitespace() || matches!(ch, '`' | '\'' | '"' | '<' | '>' | '|' | ';') {
            break;
        }
        end += 1;
    }
    let end_byte = chars
        .get(end)
        .map(|(offset, _)| *offset)
        .unwrap_or(input.len());
    let raw = input[start_byte..end_byte].trim_end_matches([',', ':', '!', '?', ')', ']', '}']);
    Ok((PathBuf::from(raw), end))
}

fn backtick_run(chars: &[(usize, char)], start: usize) -> usize {
    chars[start..]
        .iter()
        .take_while(|(_, ch)| *ch == '`')
        .count()
}

fn reference_boundary(input: &str, byte_index: usize) -> bool {
    input[..byte_index]
        .chars()
        .next_back()
        .is_none_or(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | ':' | ','))
}

fn reject_symlink_components(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("chat_attachment_path_outside_workspace"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => anyhow::bail!("chat_attachment_path_outside_workspace"),
            Component::Normal(part) => current.push(part),
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("chat_attachment_path_outside_workspace")
            }
        }
        if fs::symlink_metadata(&current)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            anyhow::bail!("chat_attachment_symlink_denied");
        }
    }
    Ok(())
}

fn reject_sensitive_path(path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name == ".env"
        || name.starts_with(".env.")
        || matches!(
            extension.as_str(),
            "key" | "pem" | "p12" | "pfx" | "keystore"
        )
    {
        anyhow::bail!("chat_attachment_sensitive_path_denied");
    }
    Ok(())
}

struct DetectedContent {
    kind: &'static str,
    mime_type: String,
}

fn detect_content(bytes: &[u8], path: &Path) -> DetectedContent {
    let image_mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    if let Some(mime_type) = image_mime {
        return DetectedContent {
            kind: "image",
            mime_type: mime_type.to_string(),
        };
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime_type = if std::str::from_utf8(bytes).is_ok() && !bytes.contains(&0) {
        match extension.as_str() {
            "md" | "markdown" => "text/markdown",
            "json" => "application/json",
            "toml" => "application/toml",
            "csv" => "text/csv",
            "html" | "htm" => "text/html",
            _ => "text/plain",
        }
    } else if bytes.starts_with(b"%PDF-") {
        "application/pdf"
    } else {
        match extension.as_str() {
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            _ => "application/octet-stream",
        }
    };
    DetectedContent {
        kind: "file",
        mime_type: mime_type.to_string(),
    }
}

#[cfg(test)]
#[path = "chat_attachments_tests.rs"]
mod tests;
