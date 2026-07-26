use std::path::Path;

use serde_json::{json, Value};

pub(super) fn content_handoff(
    workspace_root: &Path,
    requested_path: &str,
    resolved_path: &Path,
    bytes: Option<&[u8]>,
) -> Value {
    let detected = detect_content(resolved_path, bytes);
    let (capability_ref, argument_name) = match detected.kind {
        "image" => ("image_vision.describe", "images"),
        "pdf" | "document" => ("document.parse", "path"),
        _ => ("filesystem.stat_paths", "paths"),
    };
    json!({
        "schema_version": 1,
        "kind": "capability_handoff",
        "detected_kind": detected.kind,
        "mime_type": detected.mime_type,
        "capability_ref": capability_ref,
        "argument_name": argument_name,
        "reference": safe_path_reference(workspace_root, requested_path, resolved_path),
    })
}

struct DetectedContent {
    kind: &'static str,
    mime_type: &'static str,
}

fn detect_content(path: &Path, bytes: Option<&[u8]>) -> DetectedContent {
    if let Some(bytes) = bytes {
        if bytes.starts_with(b"%PDF-") {
            return DetectedContent {
                kind: "pdf",
                mime_type: "application/pdf",
            };
        }
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return image("image/png");
        }
        if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            return image("image/jpeg");
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return image("image/gif");
        }
        if bytes.starts_with(b"BM") {
            return image("image/bmp");
        }
        if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            return image("image/webp");
        }
        if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            return image("image/tiff");
        }
    }

    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => DetectedContent {
            kind: "pdf",
            mime_type: "application/pdf",
        },
        "docx" => DetectedContent {
            kind: "document",
            mime_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        },
        "md" => DetectedContent {
            kind: "document",
            mime_type: "text/markdown",
        },
        "txt" => DetectedContent {
            kind: "document",
            mime_type: "text/plain",
        },
        "html" | "htm" => DetectedContent {
            kind: "document",
            mime_type: "text/html",
        },
        "png" => image("image/png"),
        "jpg" | "jpeg" => image("image/jpeg"),
        "gif" => image("image/gif"),
        "bmp" => image("image/bmp"),
        "webp" => image("image/webp"),
        "tif" | "tiff" => image("image/tiff"),
        "svg" => image("image/svg+xml"),
        "ico" => image("image/x-icon"),
        "heic" => image("image/heic"),
        "heif" => image("image/heif"),
        _ => DetectedContent {
            kind: "binary",
            mime_type: "application/octet-stream",
        },
    }
}

fn image(mime_type: &'static str) -> DetectedContent {
    DetectedContent {
        kind: "image",
        mime_type,
    }
}

fn safe_path_reference(workspace_root: &Path, requested_path: &str, resolved_path: &Path) -> Value {
    let canonical_workspace = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.into());
    match resolved_path.strip_prefix(&canonical_workspace) {
        Ok(relative) => json!({
            "kind": "workspace_path",
            "path": relative.to_string_lossy(),
        }),
        Err(_) => json!({
            "kind": "explicit_path",
            "path": requested_path,
        }),
    }
}

#[cfg(test)]
#[path = "content_handoff_tests.rs"]
mod tests;
