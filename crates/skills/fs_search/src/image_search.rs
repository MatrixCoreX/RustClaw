use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::{json, Value};

use super::workspace_traversal::{to_rel, walk_collect, ScanLimits};

const IMAGE_METADATA_READ_LIMIT: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize)]
pub(super) struct ImageEntry {
    pub(super) path: String,
    pub(super) extension: String,
    pub(super) mime_type: String,
    pub(super) size_bytes: u64,
    pub(super) modified_unix_ms: Option<u128>,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
}

pub(super) struct ImageSnapshot {
    pub(super) entries: Vec<ImageEntry>,
    pub(super) scan_truncated: bool,
}

pub(super) fn default_image_extensions() -> Vec<String> {
    [
        "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "svg", "ico", "heic", "heif",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub(super) fn collect_images(
    workspace_root: &Path,
    search_root: &Path,
    scan_limits: ScanLimits,
    extensions: &[String],
    snapshot_limit: usize,
) -> Result<ImageSnapshot, String> {
    let mut entries = Vec::new();
    let stats = walk_collect(search_root, scan_limits, &mut |path| {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extensions.iter().any(|candidate| candidate == &extension) {
            entries.push(image_entry(workspace_root, path, extension));
        }
        entries.len() > snapshot_limit
    })?;
    let result_limit_reached = entries.len() > snapshot_limit;
    entries.truncate(snapshot_limit);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.dedup_by(|left, right| left.path == right.path);
    Ok(ImageSnapshot {
        entries,
        scan_truncated: stats.limit_reached || result_limit_reached,
    })
}

pub(super) fn directory_counts(entries: &[ImageEntry], max_dirs: usize) -> (Vec<Value>, bool) {
    let mut counts = HashMap::<String, usize>::new();
    for entry in entries {
        let directory = Path::new(&entry.path)
            .parent()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        *counts.entry(directory).or_default() += 1;
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let truncated = counts.len() > max_dirs;
    counts.truncate(max_dirs);
    (
        counts
            .into_iter()
            .map(|(directory, count)| json!({"dir": directory, "count": count}))
            .collect(),
        truncated,
    )
}

fn image_entry(workspace_root: &Path, path: &Path, extension: String) -> ImageEntry {
    let metadata = std::fs::metadata(path).ok();
    let modified_unix_ms = metadata
        .as_ref()
        .and_then(|value| value.modified().ok())
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis());
    let (width, height) = image_dimensions(path, &extension).unwrap_or((None, None));
    ImageEntry {
        path: to_rel(workspace_root, path),
        mime_type: mime_type(&extension).to_string(),
        extension,
        size_bytes: metadata.map(|value| value.len()).unwrap_or(0),
        modified_unix_ms,
        width,
        height,
    }
}

fn image_dimensions(path: &Path, extension: &str) -> Result<(Option<u32>, Option<u32>), ()> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| ())?
        .take(IMAGE_METADATA_READ_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    let dimensions = match extension {
        "png" => png_dimensions(&bytes),
        "jpg" | "jpeg" => jpeg_dimensions(&bytes),
        "gif" => gif_dimensions(&bytes),
        "bmp" => bmp_dimensions(&bytes),
        _ => None,
    };
    Ok(dimensions
        .map(|(width, height)| (Some(width), Some(height)))
        .unwrap_or((None, None)))
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    (bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24).then(|| {
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap_or_default()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap_or_default()),
        )
    })
}

fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    ((bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 10).then(
        || {
            (
                u16::from_le_bytes(bytes[6..8].try_into().unwrap_or_default()) as u32,
                u16::from_le_bytes(bytes[8..10].try_into().unwrap_or_default()) as u32,
            )
        },
    )
}

fn bmp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    (bytes.starts_with(b"BM") && bytes.len() >= 26).then(|| {
        (
            u32::from_le_bytes(bytes[18..22].try_into().unwrap_or_default()),
            u32::from_le_bytes(bytes[22..26].try_into().unwrap_or_default()),
        )
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut cursor = 2usize;
    while cursor + 4 <= bytes.len() {
        while cursor < bytes.len() && bytes[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *bytes.get(cursor)?;
        cursor += 1;
        if matches!(marker, 0xd8 | 0xd9) {
            continue;
        }
        let length = u16::from_be_bytes(bytes.get(cursor..cursor + 2)?.try_into().ok()?) as usize;
        if length < 2 || cursor.saturating_add(length) > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) && length >= 7
        {
            let height =
                u16::from_be_bytes(bytes.get(cursor + 3..cursor + 5)?.try_into().ok()?) as u32;
            let width =
                u16::from_be_bytes(bytes.get(cursor + 5..cursor + 7)?.try_into().ok()?) as u32;
            return Some((width, height));
        }
        cursor += length;
    }
    None
}

fn mime_type(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "heic" => "image/heic",
        "heif" => "image/heif",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[path = "image_search_tests.rs"]
mod tests;
