//! Shared outbound channel media limits and local-file preflight checks.
//!
//! These limits are intentionally checked before a channel adapter reads an
//! entire file into memory or starts a remote upload.

use std::path::Path;

pub const MIB: u64 = 1024 * 1024;

pub const TELEGRAM_IMAGE_MAX_BYTES: u64 = 10 * MIB;
pub const TELEGRAM_OTHER_MAX_BYTES: u64 = 50 * MIB;

pub const WHATSAPP_CLOUD_IMAGE_MAX_BYTES: u64 = 5 * MIB;
pub const WHATSAPP_CLOUD_VIDEO_MAX_BYTES: u64 = 16 * MIB;
pub const WHATSAPP_CLOUD_AUDIO_MAX_BYTES: u64 = 16 * MIB;
pub const WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES: u64 = 100 * MIB;

pub const FEISHU_LARK_IMAGE_MAX_BYTES: u64 = 10 * MIB;
pub const FEISHU_LARK_FILE_MAX_BYTES: u64 = 30 * MIB;

// WeChat iLink does not publish a stable public outbound bot-media contract.
// Keep outbound guards aligned with this repository's existing inbound safety
// policy instead of presenting them as upstream API guarantees.
pub const WECHAT_IMAGE_SAFETY_MAX_BYTES: u64 = 25 * MIB;
pub const WECHAT_OTHER_SAFETY_MAX_BYTES: u64 = 100 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsappCloudMediaKind {
    Image,
    Video,
    Audio,
    Document,
}

pub fn whatsapp_cloud_upload_spec(
    path: &Path,
    kind: WhatsappCloudMediaKind,
) -> Result<(&'static str, u64, &'static str), String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let spec = match kind {
        WhatsappCloudMediaKind::Image => match extension.as_str() {
            "jpg" | "jpeg" => ("image/jpeg", WHATSAPP_CLOUD_IMAGE_MAX_BYTES, "图片"),
            "png" => ("image/png", WHATSAPP_CLOUD_IMAGE_MAX_BYTES, "图片"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 图片格式不支持：.{extension}。仅支持 JPEG 和 PNG。"
                ))
            }
        },
        WhatsappCloudMediaKind::Video => match extension.as_str() {
            "mp4" => ("video/mp4", WHATSAPP_CLOUD_VIDEO_MAX_BYTES, "视频"),
            "3gp" | "3gpp" => ("video/3gpp", WHATSAPP_CLOUD_VIDEO_MAX_BYTES, "视频"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 视频格式不支持：.{extension}。仅支持 MP4/3GP，且视频需为 H.264、音频需为 AAC。"
                ))
            }
        },
        WhatsappCloudMediaKind::Audio => match extension.as_str() {
            "aac" => ("audio/aac", WHATSAPP_CLOUD_AUDIO_MAX_BYTES, "音频"),
            "m4a" | "mp4" => ("audio/mp4", WHATSAPP_CLOUD_AUDIO_MAX_BYTES, "音频"),
            "mp3" => ("audio/mpeg", WHATSAPP_CLOUD_AUDIO_MAX_BYTES, "音频"),
            "amr" => ("audio/amr", WHATSAPP_CLOUD_AUDIO_MAX_BYTES, "音频"),
            "ogg" | "opus" => ("audio/ogg", WHATSAPP_CLOUD_AUDIO_MAX_BYTES, "音频"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 音频格式不支持：.{extension}。支持 AAC、M4A、MP3、AMR 和 Opus OGG。"
                ))
            }
        },
        WhatsappCloudMediaKind::Document => match extension.as_str() {
            "txt" => ("text/plain", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "pdf" => ("application/pdf", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "ppt" => ("application/vnd.ms-powerpoint", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "doc" => ("application/msword", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "xls" => ("application/vnd.ms-excel", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "docx" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "pptx" => ("application/vnd.openxmlformats-officedocument.presentationml.presentation", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            "xlsx" => ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", WHATSAPP_CLOUD_DOCUMENT_MAX_BYTES, "文件"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 文件格式不支持：.{extension}。支持 TXT、PDF、PPT/PPTX、DOC/DOCX 和 XLS/XLSX。"
                ))
            }
        },
    };
    Ok(spec)
}

pub fn validate_local_media_file(
    path: &Path,
    channel: &str,
    media_kind: &str,
    max_bytes: u64,
) -> Result<u64, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| format!("{channel} {media_kind}文件无法读取：{err}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "{channel} {media_kind}投送失败：{} 不是普通文件",
            path.display()
        ));
    }
    let actual_bytes = metadata.len();
    if actual_bytes == 0 {
        return Err(format!(
            "{channel} {media_kind}投送失败：{} 是空文件",
            path.display()
        ));
    }
    if actual_bytes > max_bytes {
        return Err(format!(
            "{channel} {media_kind}过大：{:.2} MiB，平台上限为 {:.0} MiB。请压缩后重试，或改为在 UI 中下载原文件。",
            actual_bytes as f64 / MIB as f64,
            max_bytes as f64 / MIB as f64
        ));
    }
    Ok(actual_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_oversized_files_before_upload() {
        let dir =
            std::env::temp_dir().join(format!("channel-media-limit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let empty = dir.join("empty.bin");
        std::fs::write(&empty, []).expect("write empty file");
        assert!(validate_local_media_file(&empty, "test", "文件", 10)
            .unwrap_err()
            .contains("空文件"));

        let large = dir.join("large.bin");
        let file = std::fs::File::create(&large).expect("create sparse file");
        file.set_len(11).expect("set sparse length");
        let error = validate_local_media_file(&large, "test", "视频", 10).unwrap_err();
        assert!(error.contains("过大"));
        assert!(error.contains("UI"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
