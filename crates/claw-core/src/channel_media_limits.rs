//! Shared outbound channel media limits and local-file preflight checks.
//!
//! These limits are intentionally checked before a channel adapter reads an
//! entire file into memory or starts a remote upload.

use std::path::Path;

use crate::channel_capabilities::{
    channel_capability, channel_media_max_bytes, ChannelAdapterKind, ChannelCapabilityKind,
};

pub use crate::channel_capabilities::MIB;

pub fn required_channel_media_max_bytes(
    adapter: ChannelAdapterKind,
    capability: ChannelCapabilityKind,
) -> u64 {
    channel_media_max_bytes(adapter, capability)
        .expect("every executable outbound media path must have a catalog limit")
}

pub fn telegram_image_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendImage,
    )
}

pub fn telegram_file_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendFile,
    )
}

pub fn feishu_image_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendImage,
    )
}

pub fn feishu_file_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendFile,
    )
}

pub fn lark_image_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendImage,
    )
}

pub fn lark_file_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendFile,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalMediaPreflightFailure {
    Unreadable,
    NotRegularFile,
    Empty,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMediaPreflightError {
    pub failure: LocalMediaPreflightFailure,
    pub actual_bytes: Option<u64>,
    pub max_bytes: u64,
    diagnostic: Option<String>,
}

impl LocalMediaPreflightError {
    pub fn error_code(&self) -> &'static str {
        match self.failure {
            LocalMediaPreflightFailure::Unreadable => "channel_media_unreadable",
            LocalMediaPreflightFailure::NotRegularFile => "channel_media_not_regular_file",
            LocalMediaPreflightFailure::Empty => "channel_media_empty",
            LocalMediaPreflightFailure::TooLarge => "channel_media_too_large",
        }
    }

    pub fn message_key(&self) -> &'static str {
        match self.failure {
            LocalMediaPreflightFailure::Unreadable => "channel.media.preflight.unreadable",
            LocalMediaPreflightFailure::NotRegularFile => {
                "channel.media.preflight.not_regular_file"
            }
            LocalMediaPreflightFailure::Empty => "channel.media.preflight.empty",
            LocalMediaPreflightFailure::TooLarge => "channel.media.preflight.too_large",
        }
    }
}

pub fn preflight_local_media_file(
    path: &Path,
    max_bytes: u64,
) -> Result<u64, LocalMediaPreflightError> {
    let metadata = std::fs::metadata(path).map_err(|error| LocalMediaPreflightError {
        failure: LocalMediaPreflightFailure::Unreadable,
        actual_bytes: None,
        max_bytes,
        diagnostic: Some(error.to_string()),
    })?;
    if !metadata.is_file() {
        return Err(LocalMediaPreflightError {
            failure: LocalMediaPreflightFailure::NotRegularFile,
            actual_bytes: None,
            max_bytes,
            diagnostic: None,
        });
    }
    let actual_bytes = metadata.len();
    if actual_bytes == 0 {
        return Err(LocalMediaPreflightError {
            failure: LocalMediaPreflightFailure::Empty,
            actual_bytes: Some(actual_bytes),
            max_bytes,
            diagnostic: None,
        });
    }
    if actual_bytes > max_bytes {
        return Err(LocalMediaPreflightError {
            failure: LocalMediaPreflightFailure::TooLarge,
            actual_bytes: Some(actual_bytes),
            max_bytes,
            diagnostic: None,
        });
    }
    Ok(actual_bytes)
}

pub fn wechat_image_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendImage,
    )
}

pub fn wechat_file_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendFile,
    )
}

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
    let capability_kind = match kind {
        WhatsappCloudMediaKind::Image => ChannelCapabilityKind::SendImage,
        WhatsappCloudMediaKind::Video => ChannelCapabilityKind::SendVideo,
        WhatsappCloudMediaKind::Audio => ChannelCapabilityKind::SendAudio,
        WhatsappCloudMediaKind::Document => ChannelCapabilityKind::SendFile,
    };
    let record = channel_capability(ChannelAdapterKind::WhatsappCloud, capability_kind)
        .ok_or_else(|| "channel_capability_catalog_missing".to_string())?;
    let max_bytes = record
        .max_payload_bytes
        .ok_or_else(|| "channel_capability_limit_missing".to_string())?;
    let spec = match kind {
        WhatsappCloudMediaKind::Image => match extension.as_str() {
            "jpg" | "jpeg" => ("image/jpeg", max_bytes, "图片"),
            "png" => ("image/png", max_bytes, "图片"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 图片格式不支持：.{extension}。仅支持 JPEG 和 PNG。"
                ))
            }
        },
        WhatsappCloudMediaKind::Video => match extension.as_str() {
            "mp4" => ("video/mp4", max_bytes, "视频"),
            "3gp" | "3gpp" => ("video/3gpp", max_bytes, "视频"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 视频格式不支持：.{extension}。仅支持 MP4/3GP，且视频需为 H.264、音频需为 AAC。"
                ))
            }
        },
        WhatsappCloudMediaKind::Audio => match extension.as_str() {
            "aac" => ("audio/aac", max_bytes, "音频"),
            "m4a" | "mp4" => ("audio/mp4", max_bytes, "音频"),
            "mp3" => ("audio/mpeg", max_bytes, "音频"),
            "amr" => ("audio/amr", max_bytes, "音频"),
            "ogg" | "opus" => ("audio/ogg", max_bytes, "音频"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 音频格式不支持：.{extension}。支持 AAC、M4A、MP3、AMR 和 Opus OGG。"
                ))
            }
        },
        WhatsappCloudMediaKind::Document => match extension.as_str() {
            "txt" => ("text/plain", max_bytes, "文件"),
            "pdf" => ("application/pdf", max_bytes, "文件"),
            "ppt" => ("application/vnd.ms-powerpoint", max_bytes, "文件"),
            "doc" => ("application/msword", max_bytes, "文件"),
            "xls" => ("application/vnd.ms-excel", max_bytes, "文件"),
            "docx" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", max_bytes, "文件"),
            "pptx" => ("application/vnd.openxmlformats-officedocument.presentationml.presentation", max_bytes, "文件"),
            "xlsx" => ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", max_bytes, "文件"),
            _ => {
                return Err(format!(
                    "WhatsApp Cloud 文件格式不支持：.{extension}。支持 TXT、PDF、PPT/PPTX、DOC/DOCX 和 XLS/XLSX。"
                ))
            }
        },
    };
    if !record.accepted_mime_types.contains(&spec.0) {
        return Err("channel_capability_mime_mismatch".to_string());
    }
    Ok(spec)
}

pub fn validate_local_media_file(
    path: &Path,
    channel: &str,
    media_kind: &str,
    max_bytes: u64,
) -> Result<u64, String> {
    match preflight_local_media_file(path, max_bytes) {
        Ok(actual_bytes) => Ok(actual_bytes),
        Err(error) => match error.failure {
            LocalMediaPreflightFailure::Unreadable => Err(format!(
                "{channel} {media_kind}文件无法读取：{}",
                error.diagnostic.as_deref().unwrap_or(error.error_code())
            )),
            LocalMediaPreflightFailure::NotRegularFile => Err(format!(
                "{channel} {media_kind}投送失败：{} 不是普通文件",
                path.display()
            )),
            LocalMediaPreflightFailure::Empty => Err(format!(
                "{channel} {media_kind}投送失败：{} 是空文件",
                path.display()
            )),
            LocalMediaPreflightFailure::TooLarge => Err(format!(
                "{channel} {media_kind}过大：{:.2} MiB，平台上限为 {:.0} MiB。请压缩后重试，或改为在 UI 中下载原文件。",
                error.actual_bytes.unwrap_or_default() as f64 / MIB as f64,
                max_bytes as f64 / MIB as f64
            )),
        },
    }
}

#[cfg(test)]
#[path = "channel_media_limits_tests.rs"]
mod tests;
