//! Shared outbound channel media limits and local-file preflight checks.
//!
//! These limits are intentionally checked before a channel adapter reads an
//! entire file into memory or starts a remote upload.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;

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
    let metadata = std::fs::metadata(path).map_err(|_| LocalMediaPreflightError {
        failure: LocalMediaPreflightFailure::Unreadable,
        actual_bytes: None,
        max_bytes,
    })?;
    if !metadata.is_file() {
        return Err(LocalMediaPreflightError {
            failure: LocalMediaPreflightFailure::NotRegularFile,
            actual_bytes: None,
            max_bytes,
        });
    }
    let actual_bytes = metadata.len();
    if actual_bytes == 0 {
        return Err(LocalMediaPreflightError {
            failure: LocalMediaPreflightFailure::Empty,
            actual_bytes: Some(actual_bytes),
            max_bytes,
        });
    }
    if actual_bytes > max_bytes {
        return Err(LocalMediaPreflightError {
            failure: LocalMediaPreflightFailure::TooLarge,
            actual_bytes: Some(actual_bytes),
            max_bytes,
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

pub fn wechat_video_max_bytes() -> u64 {
    required_channel_media_max_bytes(
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendVideo,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhatsappCloudMediaKind {
    Image,
    Video,
    Audio,
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWhatsappCloudMedia {
    pub path: PathBuf,
    pub mime_type: &'static str,
    pub max_bytes: u64,
    pub media_label: &'static str,
    pub compatible_copy_created: bool,
}

#[derive(Debug, Deserialize)]
struct MediaProbe {
    #[serde(default)]
    streams: Vec<MediaProbeStream>,
}

#[derive(Debug, Deserialize)]
struct MediaProbeStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
}

pub async fn prepare_whatsapp_cloud_media(
    original_path: &Path,
    kind: WhatsappCloudMediaKind,
    compatible_output_dir: &Path,
) -> Result<PreparedWhatsappCloudMedia, String> {
    let capability = match kind {
        WhatsappCloudMediaKind::Image => ChannelCapabilityKind::SendImage,
        WhatsappCloudMediaKind::Video => ChannelCapabilityKind::SendVideo,
        WhatsappCloudMediaKind::Audio => ChannelCapabilityKind::SendAudio,
        WhatsappCloudMediaKind::Document => ChannelCapabilityKind::SendFile,
    };
    let input_max_bytes =
        required_channel_media_max_bytes(ChannelAdapterKind::WhatsappCloud, capability);
    preflight_local_media_file(original_path, input_max_bytes).map_err(|error| {
        format!(
            "whatsapp_cloud_media_preflight_failed:{}:{}:{}",
            error.error_code(),
            error.actual_bytes.unwrap_or_default(),
            error.max_bytes
        )
    })?;
    let extension = original_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let needs_compatible_copy = match kind {
        WhatsappCloudMediaKind::Image => !matches!(extension.as_str(), "jpg" | "jpeg" | "png"),
        WhatsappCloudMediaKind::Video => {
            !matches!(extension.as_str(), "mp4" | "3gp" | "3gpp")
                || !whatsapp_video_codecs_are_compatible(original_path).await?
        }
        WhatsappCloudMediaKind::Audio => {
            if extension == "ogg" || extension == "opus" {
                !whatsapp_ogg_audio_is_opus(original_path).await?
            } else {
                !matches!(extension.as_str(), "aac" | "m4a" | "mp4" | "mp3" | "amr")
            }
        }
        WhatsappCloudMediaKind::Document => false,
    };
    let (path, compatible_copy_created) = if needs_compatible_copy {
        let output_extension = match kind {
            WhatsappCloudMediaKind::Image => "jpg",
            WhatsappCloudMediaKind::Video => "mp4",
            WhatsappCloudMediaKind::Audio => "m4a",
            WhatsappCloudMediaKind::Document => {
                return Err("whatsapp_cloud_document_conversion_unsupported".to_string())
            }
        };
        tokio::fs::create_dir_all(compatible_output_dir)
            .await
            .map_err(|_| "whatsapp_cloud_compatible_output_create_failed".to_string())?;
        let output_path = compatible_output_dir.join(format!(
            "compatible-{}.{}",
            uuid::Uuid::new_v4().simple(),
            output_extension
        ));
        create_whatsapp_compatible_copy(original_path, &output_path, kind).await?;
        (output_path, true)
    } else {
        (original_path.to_path_buf(), false)
    };
    let (mime_type, max_bytes, media_label) = match whatsapp_cloud_upload_spec(&path, kind) {
        Ok(spec) => spec,
        Err(error) => {
            cleanup_compatible_copy(&path, compatible_copy_created).await;
            return Err(error);
        }
    };
    if let Err(error) = preflight_local_media_file(&path, max_bytes) {
        cleanup_compatible_copy(&path, compatible_copy_created).await;
        return Err(format!(
            "whatsapp_cloud_media_preflight_failed:{}:{}:{}",
            error.error_code(),
            error.actual_bytes.unwrap_or_default(),
            error.max_bytes
        ));
    }
    let video_compatible = if kind == WhatsappCloudMediaKind::Video {
        match whatsapp_video_codecs_are_compatible(&path).await {
            Ok(value) => value,
            Err(error) => {
                cleanup_compatible_copy(&path, compatible_copy_created).await;
                return Err(error);
            }
        }
    } else {
        true
    };
    if !video_compatible {
        if compatible_copy_created {
            let _ = tokio::fs::remove_file(&path).await;
        }
        return Err("whatsapp_cloud_video_codec_incompatible".to_string());
    }
    Ok(PreparedWhatsappCloudMedia {
        path,
        mime_type,
        max_bytes,
        media_label,
        compatible_copy_created,
    })
}

async fn cleanup_compatible_copy(path: &Path, compatible_copy_created: bool) {
    if compatible_copy_created {
        let _ = tokio::fs::remove_file(path).await;
    }
}

async fn media_probe(path: &Path) -> Result<MediaProbe, String> {
    let mut command = tokio::process::Command::new("ffprobe");
    command
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("stream=codec_type,codec_name")
        .arg("-of")
        .arg("json")
        .arg(path)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| "whatsapp_cloud_media_probe_timeout".to_string())?
        .map_err(|_| "whatsapp_cloud_media_probe_unavailable".to_string())?;
    if !output.status.success() {
        return Err("whatsapp_cloud_media_probe_failed".to_string());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "whatsapp_cloud_media_probe_invalid".to_string())
}

async fn whatsapp_video_codecs_are_compatible(path: &Path) -> Result<bool, String> {
    let probe = media_probe(path).await?;
    Ok(video_probe_is_compatible(&probe))
}

fn video_probe_is_compatible(probe: &MediaProbe) -> bool {
    let has_h264_video = probe
        .streams
        .iter()
        .any(|stream| stream.codec_type == "video" && stream.codec_name == "h264");
    let video_streams_valid = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type == "video")
        .all(|stream| stream.codec_name == "h264");
    let audio_streams_valid = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type == "audio")
        .all(|stream| stream.codec_name == "aac");
    has_h264_video && video_streams_valid && audio_streams_valid
}

async fn whatsapp_ogg_audio_is_opus(path: &Path) -> Result<bool, String> {
    let probe = media_probe(path).await?;
    Ok(probe
        .streams
        .iter()
        .any(|stream| stream.codec_type == "audio" && stream.codec_name == "opus"))
}

async fn create_whatsapp_compatible_copy(
    original_path: &Path,
    output_path: &Path,
    kind: WhatsappCloudMediaKind,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .arg("-nostdin")
        .arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(original_path);
    match kind {
        WhatsappCloudMediaKind::Image => {
            command.arg("-frames:v").arg("1").arg("-q:v").arg("2");
        }
        WhatsappCloudMediaKind::Video => {
            command
                .arg("-c:v")
                .arg("libx264")
                .arg("-pix_fmt")
                .arg("yuv420p")
                .arg("-c:a")
                .arg("aac")
                .arg("-movflags")
                .arg("+faststart");
        }
        WhatsappCloudMediaKind::Audio => {
            command.arg("-vn").arg("-c:a").arg("aac");
        }
        WhatsappCloudMediaKind::Document => {
            return Err("whatsapp_cloud_document_conversion_unsupported".to_string())
        }
    }
    command
        .arg(output_path)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(120), command.output())
        .await
        .map_err(|_| "whatsapp_cloud_media_conversion_timeout".to_string())?
        .map_err(|_| "whatsapp_cloud_media_conversion_unavailable".to_string())?;
    if !output.status.success() {
        let _ = tokio::fs::remove_file(output_path).await;
        return Err("whatsapp_cloud_media_conversion_failed".to_string());
    }
    Ok(())
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
            "jpg" | "jpeg" => ("image/jpeg", max_bytes, "image"),
            "png" => ("image/png", max_bytes, "image"),
            _ => {
                return Err(format!(
                    "whatsapp_cloud_image_format_unsupported:{extension}"
                ))
            }
        },
        WhatsappCloudMediaKind::Video => match extension.as_str() {
            "mp4" => ("video/mp4", max_bytes, "video"),
            "3gp" | "3gpp" => ("video/3gpp", max_bytes, "video"),
            _ => {
                return Err(format!(
                    "whatsapp_cloud_video_format_unsupported:{extension}"
                ))
            }
        },
        WhatsappCloudMediaKind::Audio => match extension.as_str() {
            "aac" => ("audio/aac", max_bytes, "audio"),
            "m4a" | "mp4" => ("audio/mp4", max_bytes, "audio"),
            "mp3" => ("audio/mpeg", max_bytes, "audio"),
            "amr" => ("audio/amr", max_bytes, "audio"),
            "ogg" | "opus" => ("audio/ogg", max_bytes, "audio"),
            _ => {
                return Err(format!(
                    "whatsapp_cloud_audio_format_unsupported:{extension}"
                ))
            }
        },
        WhatsappCloudMediaKind::Document => match extension.as_str() {
            "txt" => ("text/plain", max_bytes, "document"),
            "pdf" => ("application/pdf", max_bytes, "document"),
            "ppt" => ("application/vnd.ms-powerpoint", max_bytes, "document"),
            "doc" => ("application/msword", max_bytes, "document"),
            "xls" => ("application/vnd.ms-excel", max_bytes, "document"),
            "docx" => (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                max_bytes,
                "document",
            ),
            "pptx" => (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                max_bytes,
                "document",
            ),
            "xlsx" => (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                max_bytes,
                "document",
            ),
            _ => {
                return Err(format!(
                    "whatsapp_cloud_document_format_unsupported:{extension}"
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
    _channel: &str,
    _media_kind: &str,
    max_bytes: u64,
) -> Result<u64, String> {
    preflight_local_media_file(path, max_bytes).map_err(|error| {
        format!(
            "channel_media_preflight_failed:{}:{}:{}",
            error.error_code(),
            error.actual_bytes.unwrap_or_default(),
            error.max_bytes
        )
    })
}

#[cfg(test)]
#[path = "channel_media_limits_tests.rs"]
mod tests;
